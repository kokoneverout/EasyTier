//! The NRPT catch-all belongs to the TUN lifetime, not to a scheduled task.
use std::{io, net::IpAddr};

pub(crate) trait PolicyBackend {
    fn apply(&mut self, server: IpAddr) -> io::Result<()>;
    fn clear(&mut self) -> io::Result<()>;
}

pub(crate) struct ExitDnsPolicy<B: PolicyBackend> {
    backend: B,
}

impl<B: PolicyBackend> ExitDnsPolicy<B> {
    fn install(backend: B, server: IpAddr) -> io::Result<Self> {
        // Construct before applying so a partial installation is also undone.
        let mut policy = Self { backend };
        policy.backend.apply(server)?;
        tracing::info!(%server, "exit DNS policy installed");
        Ok(policy)
    }
}

impl<B: PolicyBackend> Drop for ExitDnsPolicy<B> {
    fn drop(&mut self) {
        match self.backend.clear() {
            Ok(()) => tracing::info!("exit DNS policy removed"),
            Err(error) => tracing::error!(?error, "failed to remove exit DNS policy"),
        }
    }
}

#[cfg(windows)]
mod native {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows::{
        Win32::{
            Foundation::{HANDLE, WAIT_OBJECT_0},
            System::Threading::{CreateSemaphoreW, ReleaseSemaphore, WaitForSingleObject},
        },
        core::w,
    };
    use winreg::{RegKey, enums::*};

    use super::*;
    use crate::instance::dns_server::system_config::windows::{WindowsDNSManager, flush_dns};

    const ROOT: &str = r"SYSTEM\CurrentControlSet\Services\Dnscache\Parameters\DnsPolicyConfig";
    const GROUP_POLICY_ROOT: &str =
        r"SOFTWARE\Policies\Microsoft\Windows NT\DNSClient\DnsPolicyConfig";
    const KEY: &str = "{AB00BCCE-56EB-4CFE-83B3-57891CC5936D}";
    const OWNER_PREFIX: &str = "EasyTier:exit-dns:";

    // A semaphore can be released on any runtime thread. A Win32 mutex cannot.
    struct PolicyPermit(OwnedHandle);

    impl PolicyPermit {
        fn acquire() -> io::Result<Self> {
            let raw = unsafe { CreateSemaphoreW(None, 1, 1, w!("Global\\EasyTierExitDnsPolicy")) }
                .map_err(io::Error::other)?;
            let handle = unsafe { OwnedHandle::from_raw_handle(raw.0) };
            if unsafe { WaitForSingleObject(raw, 0) } != WAIT_OBJECT_0 {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "another EasyTier instance owns the exit DNS policy",
                ));
            }
            Ok(Self(handle))
        }
    }

    impl Drop for PolicyPermit {
        fn drop(&mut self) {
            let _ = unsafe { ReleaseSemaphore(HANDLE(self.0.as_raw_handle()), 1, None) };
        }
    }

    pub(crate) struct NativePolicy {
        root: RegKey,
        owner: String,
        dns: WindowsDNSManager,
        created: bool,
        _permit: PolicyPermit,
    }

    impl NativePolicy {
        fn new(tun_name: &str, instance_id: uuid::Uuid) -> io::Result<Self> {
            let permit = PolicyPermit::acquire()?;
            let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
            match hklm.open_subkey(GROUP_POLICY_ROOT) {
                Ok(key) if key.enum_keys().next().is_some() => {
                    return Err(io::Error::other(
                        "group-policy NRPT is active; local exit DNS policy cannot override it",
                    ));
                }
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            let (root, _) = hklm.create_subkey(ROOT)?;
            for name in root.enum_keys() {
                let name = name?;
                let key = root.open_subkey(&name)?;
                if name == KEY {
                    let comment: String = key.get_value("Comment")?;
                    if !comment.starts_with(OWNER_PREFIX) {
                        return Err(io::Error::other(
                            "exit DNS policy key belongs to another owner",
                        ));
                    }
                } else if key
                    .get_value::<Vec<String>, _>("Name")?
                    .iter()
                    .any(|s| s == ".")
                {
                    return Err(io::Error::other(
                        "another global NRPT rule is active; remove the old exit DNS task's rule before enabling vpn_dns",
                    ));
                }
            }
            // A killed process cannot run Drop. Reclaim only our fixed policy key.
            match root.delete_subkey_all(KEY) {
                Ok(()) => {
                    flush_dns()?;
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            Ok(Self {
                root,
                owner: format!("{OWNER_PREFIX}{instance_id}"),
                dns: WindowsDNSManager::new(tun_name)?,
                created: false,
                _permit: permit,
            })
        }
    }

    impl PolicyBackend for NativePolicy {
        fn apply(&mut self, server: IpAddr) -> io::Result<()> {
            let (key, _) = self.root.create_subkey(KEY)?;
            self.created = true;
            key.set_value("Comment", &self.owner)?;
            key.set_value("Name", &vec![".".to_string()])?;
            key.set_value("GenericDNSServers", &server.to_string())?;
            key.set_value("ConfigOptions", &8u32)?;
            key.set_value("DisplayName", &"EasyTier exit DNS")?;
            // Write the version last, after the resolver and namespace exist.
            key.set_value("Version", &2u32)?;
            self.dns.set_primary_dns(&[server], &[])
        }

        fn clear(&mut self) -> io::Result<()> {
            if !self.created {
                return Ok(());
            }
            let key = match self.root.open_subkey(KEY) {
                Ok(key) => key,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return flush_dns(),
                Err(error) => return Err(error),
            };
            match key.get_value::<String, _>("Comment") {
                Ok(owner) if owner == self.owner => {}
                // Comment may be absent if apply failed at its first write.
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Ok(_) => return Err(io::Error::other("exit DNS policy owner changed")),
                Err(error) => return Err(error),
            }
            drop(key);
            self.root.delete_subkey_all(KEY)?;
            flush_dns()
        }
    }

    pub(crate) type WindowsExitDns = ExitDnsPolicy<NativePolicy>;

    pub(crate) fn start(
        tun_name: &str,
        instance_id: uuid::Uuid,
        server: IpAddr,
    ) -> io::Result<WindowsExitDns> {
        ExitDnsPolicy::install(NativePolicy::new(tun_name, instance_id)?, server)
    }
}

#[cfg(windows)]
pub(crate) use native::{WindowsExitDns, start};

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct MemoryPolicy {
        rules: Arc<Mutex<Vec<&'static str>>>,
        fail_after_write: bool,
    }

    impl PolicyBackend for MemoryPolicy {
        fn apply(&mut self, _: IpAddr) -> io::Result<()> {
            self.rules.lock().unwrap().push("exit");
            if self.fail_after_write {
                Err(io::Error::other("interface DNS configuration failed"))
            } else {
                Ok(())
            }
        }

        fn clear(&mut self) -> io::Result<()> {
            self.rules.lock().unwrap().retain(|rule| *rule != "exit");
            Ok(())
        }
    }

    #[test]
    fn stopping_exit_removes_only_its_policy() {
        let rules = Arc::new(Mutex::new(vec!["other VPN"]));
        let policy = ExitDnsPolicy::install(
            MemoryPolicy {
                rules: rules.clone(),
                fail_after_write: false,
            },
            "10.126.144.1".parse().unwrap(),
        )
        .unwrap();
        assert_eq!(*rules.lock().unwrap(), vec!["other VPN", "exit"]);
        drop(policy);
        assert_eq!(*rules.lock().unwrap(), vec!["other VPN"]);
    }

    #[test]
    fn failed_start_does_not_leave_a_global_policy() {
        let rules = Arc::new(Mutex::new(vec!["other VPN"]));
        let result = ExitDnsPolicy::install(
            MemoryPolicy {
                rules: rules.clone(),
                fail_after_write: true,
            },
            "10.126.144.1".parse().unwrap(),
        );
        assert!(result.is_err());
        assert_eq!(*rules.lock().unwrap(), vec!["other VPN"]);
    }
}
