package com.plugin.vpnservice

import android.app.Activity
import android.content.Intent
import android.net.VpnService
import androidx.activity.result.ActivityResult
import app.tauri.annotation.Command
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import android.os.Build
import android.webkit.WebView
import org.json.JSONObject
import androidx.activity.ComponentActivity
import androidx.activity.OnBackPressedCallback

@InvokeArg
class PingArgs {
    var value: String? = null
}

@InvokeArg
class StartVpnArgs {
    var ipv4Addr: String? = null
    var ipv6Addr: String? = null
    var routes: Array<String> = emptyArray()
    var dns: String? = null
    var disallowedApplications: Array<String> = emptyArray()
    var allowedApplications: Array<String> = emptyArray()
    var applicationMode: String? = null
    var requestId: String? = null
    var mtu: Int? = null
}

@InvokeArg
class StopVpnArgs {
    var requestId: String? = null
}

@InvokeArg
class GetAppIconArgs {
    var packageName: String? = null
}

@InvokeArg
class AppRoutingPolicyPayload {
    var schemaVersion: Int? = null
    var mode: String? = null
    var packages: Array<String> = emptyArray()
}

@InvokeArg
class SetAppRoutingPolicyArgs {
    var policy: AppRoutingPolicyPayload? = null
    var validateOnly: Boolean? = null
}

@InvokeArg
class AppRoutingPageArgs {
    var open: Boolean = false
}

@TauriPlugin
class VpnServicePlugin(private val activity: Activity) : Plugin(activity) {
    private val implementation = Example()
    private var appRoutingBackCallback: OnBackPressedCallback? = null

    override fun load(webView: WebView) {
        println("load vpn service plugin")
        TauriVpnService.triggerCallback = { event, data ->
            println("vpn: triggerCallback $event $data")
            trigger(event, data)
        }
        (activity as? ComponentActivity)?.let { owner ->
            appRoutingBackCallback = object : OnBackPressedCallback(false) {
                override fun handleOnBackPressed() {
                    trigger("app_routing_back", JSObject())
                }
            }.also { owner.onBackPressedDispatcher.addCallback(owner, it) }
        }
    }

    @Command
    fun setAppRoutingPageOpen(invoke: Invoke) {
        val args = invoke.parseArgs(AppRoutingPageArgs::class.java)
        activity.runOnUiThread {
            appRoutingBackCallback?.isEnabled = args.open
            invoke.resolve(JSObject())
        }
    }

    @Command
    fun ping(invoke: Invoke) {
        val args = invoke.parseArgs(PingArgs::class.java)

        val ret = JSObject()
        ret.put("value", implementation.pong(args.value ?: "default value :("))
        invoke.resolve(ret)
    }

    @Command
    fun prepareVpn(invoke: Invoke) {
        activity.runOnUiThread {
            println("prepare vpn in plugin")
            val it = VpnService.prepare(activity)
            if (it != null) {
                startActivityForResult(invoke, it, "onPrepareVpnResult")
                return@runOnUiThread
            }
            val ret = JSObject()
            ret.put("granted", true)
            invoke.resolve(ret)
        }
    }

    @ActivityCallback
    fun onPrepareVpnResult(invoke: Invoke, result: ActivityResult) {
        val ret = JSObject()
        ret.put("granted", result.resultCode == Activity.RESULT_OK)
        invoke.resolve(ret)
    }

    @Command
    fun startVpn(invoke: Invoke) {
        val args = invoke.parseArgs(StartVpnArgs::class.java)
        activity.runOnUiThread {
            println("start vpn in plugin, args: $args")

            // Validate the app lists before tearing down the old interface, so
            // a rejected request keeps the currently running VPN untouched.
            val mode = args.applicationMode
            if (mode != null && mode != AppRoutingPolicyStore.MODE_INCLUDE && mode != AppRoutingPolicyStore.MODE_ALL) {
                invoke.reject("invalid_app_routing_mode", "invalid_app_routing_mode")
                return@runOnUiThread
            }
            val allowedForService: Array<String>
            val disallowedForService: Array<String>
            val normalizedMode: String
            if (mode == AppRoutingPolicyStore.MODE_INCLUDE) {
                if (args.disallowedApplications.isNotEmpty()) {
                    invoke.reject("conflicting_app_lists", "conflicting_app_lists")
                    return@runOnUiThread
                }
                val resolution = try {
                    AppRoutingResolver.resolveInclude(
                        AppCatalog.getInstalledAppIdentities(activity),
                        AppCatalog.getSelfUid(activity),
                        args.allowedApplications.toList(),
                    )
                } catch (e: Exception) {
                    invoke.reject("app_policy_check_failed: ${e.message}", "app_policy_check_failed")
                    return@runOnUiThread
                }
                if (resolution.effective.packages.isEmpty()) {
                    invoke.reject("no_effective_apps", "no_effective_apps")
                    return@runOnUiThread
                }
                // The service re-resolves the persisted package list at
                // establish time; pass the normalized selection, not the
                // expanded uid groups.
                allowedForService = resolution.policy.packages.toTypedArray()
                disallowedForService = emptyArray()
                normalizedMode = AppRoutingPolicyStore.MODE_INCLUDE
            } else {
                if (args.allowedApplications.isNotEmpty()) {
                    invoke.reject("conflicting_app_lists", "conflicting_app_lists")
                    return@runOnUiThread
                }
                // "all" and legacy requests without a mode share the
                // disallowed branch; the VPN app itself is always excluded by
                // its real package name.
                allowedForService = emptyArray()
                disallowedForService = (args.disallowedApplications.toList() + activity.packageName).distinct().toTypedArray()
                normalizedMode = AppRoutingPolicyStore.MODE_ALL
            }

            val it = VpnService.prepare(activity)
            val ret = JSObject()
            if (it != null) {
                ret.put("errorMsg", "need_prepare")
            } else {
                if (TauriVpnService.interfaceEstablished) {
                    invoke.reject("vpn_already_running", "vpn_already_running")
                    return@runOnUiThread
                }
                val intent = Intent(activity, TauriVpnService::class.java)
                intent.putExtra(TauriVpnService.IPV4_ADDR, args.ipv4Addr)
                intent.putExtra(TauriVpnService.IPV6_ADDR, args.ipv6Addr)
                intent.putExtra(TauriVpnService.ROUTES, args.routes)
                intent.putExtra(TauriVpnService.DNS, args.dns)
                intent.putExtra(TauriVpnService.APPLICATION_MODE, normalizedMode)
                intent.putExtra(TauriVpnService.ALLOWED_APPLICATIONS, allowedForService)
                intent.putExtra(TauriVpnService.DISALLOWED_APPLICATIONS, disallowedForService)
                intent.putExtra(TauriVpnService.REQUEST_ID, args.requestId)
                intent.putExtra(TauriVpnService.MTU, args.mtu)

                activity.startService(intent)
            }
            invoke.resolve(ret)
        }
    }

    @Command
    fun stopVpn(invoke: Invoke) {
        val args = invoke.parseArgs(StopVpnArgs::class.java)
        activity.runOnUiThread {
            println("stop vpn in plugin")
            val service = TauriVpnService.self
            try {
                service?.stopVpn(args.requestId)
                // A system-bound VpnService may survive this call. Its old
                // interface is already closed and the same service can accept
                // a subsequent start command.
                activity.stopService(Intent(activity, TauriVpnService::class.java))
                invoke.resolve(JSObject())
            } catch (e: Exception) {
                invoke.reject(e.message ?: "vpn_stop_failed", "vpn_stop_failed")
            }
        }
    }

    @Command
    fun getVpnStatus(invoke: Invoke) {
        val ret = JSObject()
        ret.put("running", TauriVpnService.self != null && TauriVpnService.interfaceEstablished)
        ret.put("ipv4Addr", TauriVpnService.ipv4Addr)
        ret.put("ipv6Addr", TauriVpnService.ipv6Addr)
        ret.put("routes", JSArray(TauriVpnService.routes.toList()))
        ret.put("dns", TauriVpnService.dns)
        ret.put("applicationMode", TauriVpnService.applicationMode)
        ret.put("allowedApplications", JSArray(TauriVpnService.allowedApplications.toList()))
        ret.put("disallowedApplications", JSArray(TauriVpnService.disallowedApplications.toList()))
        ret.put("lockdownEnabled", isLockdownEnabled())
        TauriVpnService.effectivePolicy?.let { ret.put("effectivePolicy", it) }
        TauriVpnService.skippedPackages?.let { ret.put("skippedPackages", it) }
        invoke.resolve(ret)
    }

    @Command
    fun getInstalledApps(invoke: Invoke) {
        runOnWorkerThread(invoke) {
            val catalog = AppCatalog.getCatalog(activity)
            val apps = JSArray()
            for (app in catalog.apps) {
                apps.put(
                    JSObject()
                        .put("packageName", app.packageName)
                        .put("label", app.label)
                        .put("uid", app.uid)
                        .put("isSystem", app.isSystem)
                        .put("hasLauncher", app.hasLauncher)
                        .put("enabled", app.enabled)
                        .put("hasInternetPermission", app.hasInternetPermission)
                        .put("sharedUidPackages", JSArray(app.sharedUidPackages))
                        .put("selectable", app.selectable)
                        .apply { app.unavailableReason?.let { reason -> put("unavailableReason", reason) } }
                        .put("iconKey", app.iconKey),
                )
            }
            JSObject()
                .put("selfPackageName", catalog.selfPackageName)
                .put("selfUid", catalog.selfUid)
                .put("apps", apps)
        }
    }

    @Command
    fun getAppIcon(invoke: Invoke) {
        val args = invoke.parseArgs(GetAppIconArgs::class.java)
        val packageName = args.packageName
        if (packageName.isNullOrEmpty()) {
            invoke.reject("package_name_required", "package_name_required")
            return
        }
        runOnWorkerThread(invoke) {
            val icon = AppCatalog.getAppIcon(activity, packageName)
                ?: return@runOnWorkerThread JSObject().put("packageName", packageName)
            JSObject()
                .put("packageName", icon.packageName)
                .put("iconKey", icon.iconKey)
                .put("dataUrl", icon.dataUrl)
        }
    }

    @Command
    fun getAppRoutingPolicy(invoke: Invoke) {
        runOnWorkerThread(invoke) {
            val stored = AppRoutingPolicyStore.load(activity)
            if (stored == null) {
                JSObject()
                    .put("policy", JSONObject.NULL)
                    .put("skippedPackages", JSArray())
            } else {
                val resolution = if (stored.mode == AppRoutingPolicyStore.MODE_ALL) {
                    AppRoutingResolver.resolveAll(stored.packages)
                } else {
                    AppRoutingResolver.resolveInclude(
                        AppCatalog.getInstalledAppIdentities(activity),
                        AppCatalog.getSelfUid(activity),
                        stored.packages,
                    )
                }
                JSObject()
                    .put("policy", resolution.policy.toJson())
                    .put("effectivePolicy", JSObject.fromJSONObject(AppRoutingResolver.effectiveToJson(resolution.effective)))
                    .put("skippedPackages", JSArray.from(AppRoutingResolver.skippedToJson(resolution.skipped)))
            }
        }
    }

    @Command
    fun setAppRoutingPolicy(invoke: Invoke) {
        val args = invoke.parseArgs(SetAppRoutingPolicyArgs::class.java)
        val payload = args.policy
        if (payload == null) {
            invoke.reject("policy_required", "policy_required")
            return
        }
        runOnWorkerThread(invoke) {
            val policy = AppRoutingPolicy.normalize(
                payload.schemaVersion ?: AppRoutingPolicyStore.SCHEMA_VERSION,
                payload.mode ?: "",
                payload.packages.toList(),
            )
            val resolution = if (policy.mode == AppRoutingPolicyStore.MODE_ALL) {
                AppRoutingResolver.resolveAll(policy.packages)
            } else {
                AppRoutingResolver.resolveInclude(
                    AppCatalog.getInstalledAppIdentities(activity),
                    AppCatalog.getSelfUid(activity),
                    policy.packages,
                )
            }
            val validateOnly = args.validateOnly == true
            if (validateOnly && policy.mode == AppRoutingPolicyStore.MODE_INCLUDE && resolution.effective.packages.isEmpty()) {
                throw AppRoutingPolicyException("no_effective_apps")
            }
            if (!validateOnly) {
                AppRoutingPolicyStore.save(activity, resolution.policy)
            }
            JSObject()
                .put("policy", resolution.policy.toJson())
                .put("effectivePolicy", JSObject.fromJSONObject(AppRoutingResolver.effectiveToJson(resolution.effective)))
                .put("skippedPackages", JSArray.from(AppRoutingResolver.skippedToJson(resolution.skipped)))
        }
    }

    /**
     * Visible only as a hint that unselected apps may lose connectivity when
     * the system "block connections without VPN" switch is on (API 29+).
     */
    private fun isLockdownEnabled(): Boolean {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) {
            return false
        }
        return try {
            TauriVpnService.self?.isLockdownEnabled ?: false
        } catch (e: Exception) {
            false
        }
    }

    /** Runs block off the main thread and resolves the invoke with the returned JSObject. */
    private fun runOnWorkerThread(invoke: Invoke, block: () -> JSObject) {
        Thread {
            try {
                invoke.resolve(block())
            } catch (e: Exception) {
                val message = e.message ?: e.toString()
                invoke.reject(message, message)
            }
        }.start()
    }
}
