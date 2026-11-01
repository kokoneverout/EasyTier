use super::{api::network_config_from_toml, api_input::NetworkConfigExt, toml::*};
use easytier_proto::api::manage::{NetworkConfig, NetworkingMethod};

#[test]
fn custom_dualstack_survives_toml_and_both_api_projections() {
    for dns in ["192.0.2.53", "fd12:3456::1"] {
        let original = TomlConfig::new_from_str(&format!(
            r#"
ipv6 = "fd12:3456::8/64"
vpn_dns = "{dns}"
routes_v6 = ["::/1", "8000::/1"]
exit_nodes = ["192.0.2.1", "fd12:3456::1"]
"#
        ))
        .unwrap();
        for api in [
            NetworkConfig::new_from_config(original.clone()).unwrap(),
            network_config_from_toml(&original),
        ] {
            assert_eq!(api.virtual_ipv6.as_deref(), Some("fd12:3456::8/64"));
            assert_eq!(api.vpn_dns.as_deref(), Some(dns));
            assert_ne!(api.enable_manual_routes, Some(true));
            let restored = api.gen_config().unwrap();
            let restored = TomlConfig::new_from_str(&restored.dump()).unwrap();
            assert_eq!(restored.get_ipv6(), original.get_ipv6());
            assert_eq!(restored.get_vpn_dns(), original.get_vpn_dns());
            assert_eq!(restored.get_routes_v6(), original.get_routes_v6());
            assert_eq!(restored.get_exit_nodes(), original.get_exit_nodes());
            assert_eq!(restored.get_routes(), None);
        }
    }
}

#[test]
fn custom_dualstack_preserves_independent_manual_route_modes() {
    for routes_v6 in [vec![], vec!["::/1".to_owned()]] {
        for enabled in [None, Some(false), Some(true)] {
            let input = NetworkConfig {
                networking_method: Some(NetworkingMethod::Standalone as i32),
                routes_v6: routes_v6.clone(),
                enable_manual_routes_v6: enabled,
                ..Default::default()
            };
            let config = input.gen_config().unwrap();
            let expected = enabled.unwrap_or(!routes_v6.is_empty()).then(|| {
                routes_v6
                    .iter()
                    .map(|route| route.parse().unwrap())
                    .collect()
            });
            assert_eq!(config.get_routes_v6(), expected);
            assert_eq!(config.get_routes(), None);
            for api in [
                NetworkConfig::new_from_config(config.clone()).unwrap(),
                network_config_from_toml(&config),
            ] {
                assert_eq!(api.gen_config().unwrap().get_routes_v6(), expected);
                assert_ne!(api.enable_manual_routes, Some(true));
            }
        }
    }
    let config = TomlConfig::new_from_str("routes = []\nroutes_v6 = []").unwrap();
    let restored = NetworkConfig::new_from_config(config)
        .unwrap()
        .gen_config()
        .unwrap();
    assert_eq!(restored.get_routes(), Some(vec![]));
    assert_eq!(restored.get_routes_v6(), Some(vec![]));
}

#[test]
fn custom_dualstack_rejects_invalid_dns_and_routes() {
    for field in [
        "vpn_dns = 'invalid.example'",
        "vpn_dns = '10.0.0.1:53'",
        "routes_v6 = ['fd00::/129']",
        "routes_v6 = ['fd00::1/64']",
        "routes_v6 = ['10.0.0.0/8']",
    ] {
        assert!(TomlConfig::new_from_str(field).is_err(), "{field}");
    }
    for input in [
        NetworkConfig {
            vpn_dns: Some("invalid.example".into()),
            networking_method: Some(NetworkingMethod::Standalone as i32),
            ..Default::default()
        },
        NetworkConfig {
            virtual_ipv6: Some("10.0.0.1/24".into()),
            networking_method: Some(NetworkingMethod::Standalone as i32),
            ..Default::default()
        },
        NetworkConfig {
            routes_v6: vec!["fd00::1/64".into()],
            networking_method: Some(NetworkingMethod::Standalone as i32),
            ..Default::default()
        },
    ] {
        assert!(input.gen_config().is_err());
    }
    let old = TomlConfig::new_from_str("ipv4 = '10.0.0.1/24'").unwrap();
    assert_eq!(old.get_vpn_dns(), None);
    assert_eq!(old.get_routes_v6(), None);
}

#[cfg(feature = "browser-config")]
#[test]
fn custom_dualstack_browser_merge_updates_and_clears_managed_fields() {
    use super::api_input::merge_network_config_toml;

    let original =
        "ipv6 = 'fd00::1/64'\nvpn_dns = '10.0.0.1'\nroutes_v6 = ['::/1']\nnetns = 'preserve-me'";
    let input = NetworkConfig {
        networking_method: Some(NetworkingMethod::Standalone as i32),
        virtual_ipv6: Some("fd01::2/64".into()),
        vpn_dns: Some("fd01::1".into()),
        routes_v6: vec!["8000::/1".into()],
        ..Default::default()
    };
    let updated = merge_network_config_toml(original, &input).unwrap();
    let config = TomlConfig::new_from_str(&updated).unwrap();
    assert_eq!(config.get_ipv6(), Some("fd01::2/64".parse().unwrap()));
    assert_eq!(config.get_vpn_dns(), Some("fd01::1".parse().unwrap()));
    assert_eq!(
        config.get_routes_v6(),
        Some(vec!["8000::/1".parse().unwrap()])
    );
    assert_eq!(config.get_netns().as_deref(), Some("preserve-me"));

    let cleared = merge_network_config_toml(
        &updated,
        &NetworkConfig {
            networking_method: Some(NetworkingMethod::Standalone as i32),
            ..Default::default()
        },
    )
    .unwrap();
    let config = TomlConfig::new_from_str(&cleared).unwrap();
    assert_eq!(config.get_ipv6(), None);
    assert_eq!(config.get_vpn_dns(), None);
    assert_eq!(config.get_routes_v6(), None);
    assert_eq!(config.get_netns().as_deref(), Some("preserve-me"));
}
