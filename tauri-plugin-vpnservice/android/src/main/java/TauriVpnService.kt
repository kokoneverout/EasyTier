package com.plugin.vpnservice

import android.content.Intent
import android.net.VpnService
import android.os.Build
import android.os.Bundle
import android.os.ParcelFileDescriptor
import android.content.pm.PackageManager

import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject

/** Thrown when the VPN interface cannot be established; carries a stable code for the UI. */
class VpnEstablishException(val code: String, message: String) : Exception(message)

class TauriVpnService : VpnService() {
    companion object {
        @JvmField var triggerCallback: (String, JSObject) -> Unit = { _, _ -> }
        @JvmField @Volatile var self: TauriVpnService? = null
        @JvmField var ipv4Addr: String? = null
        @JvmField var ipv6Addr: String? = null
        @JvmField var routes: Array<String> = emptyArray()
        @JvmField var dns: String? = null

        // State of the actually-established interface (not the saved preference).
        @JvmField @Volatile var interfaceEstablished: Boolean = false
        @JvmField var applicationMode: String? = null
        @JvmField var allowedApplications: Array<String> = emptyArray()
        @JvmField var disallowedApplications: Array<String> = emptyArray()
        @JvmField var effectivePolicy: JSObject? = null
        @JvmField var skippedPackages: JSArray? = null

        const val IPV4_ADDR = "IPV4_ADDR"
        const val IPV6_ADDR = "IPV6_ADDR"
        const val ROUTES = "ROUTES"
        const val DNS = "DNS"
        const val DISALLOWED_APPLICATIONS = "DISALLOWED_APPLICATIONS"
        const val ALLOWED_APPLICATIONS = "ALLOWED_APPLICATIONS"
        const val APPLICATION_MODE = "APPLICATION_MODE"
        const val REQUEST_ID = "REQUEST_ID"
        const val MTU = "MTU"

    }

    private lateinit var vpnInterface: ParcelFileDescriptor
    private var stopRequestId: String? = null

    /** The system may keep this service bound after stopSelf. Closing the
     * interface, rather than onDestroy, is the completion boundary. The caller
     * has already awaited Rust's detach before requesting this stop. */
    fun stopVpn(requestId: String?) {
        stopRequestId = requestId
        disconnect()
        stopSelf()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        println("vpn on start command $intent")
        // Restarted by the system with an empty intent: the saved routing policy
        // alone cannot rebuild a full interface, so do not guess an all-app VPN.
        if (intent == null) {
            clearStatus()
            stopSelf()
            return START_NOT_STICKY
        }

        val args = intent.extras
        val requestId = args?.getString(REQUEST_ID)
        // The coordinator must detach and stop the old service first. Never
        // close a live FD here while Rust still owns I/O tasks for it.
        if (interfaceEstablished) {
            triggerCallback("vpn_service_error", JSObject()
                .put("requestId", requestId)
                .put("code", "vpn_already_running")
                .put("message", "vpn_already_running"))
            return START_STICKY
        }

        stopRequestId = null

        try {
            vpnInterface = createVpnInterface(args)
        } catch (e: Exception) {
            val code = if (e is VpnEstablishException) e.code else "establish_failed"
            println("vpn establish failed: $code ${e.message}")
            clearStatus()
            stopSelf()
            val data = JSObject()
            if (requestId != null) {
                data.put("requestId", requestId)
            }
            data.put("code", code)
            data.put("message", e.message ?: e.toString())
            triggerCallback("vpn_service_error", data)
            return START_NOT_STICKY
        }

        interfaceEstablished = true
        println("vpn created ${vpnInterface.fd}")

        val data = JSObject()
        data.put("fd", vpnInterface.fd)
        if (requestId != null) {
            data.put("requestId", requestId)
        }
        effectivePolicy?.let { data.put("effectivePolicy", it) }
        skippedPackages?.let { data.put("skippedPackages", it) }
        triggerCallback("vpn_service_start", data)

        return START_STICKY
    }

    override fun onCreate() {
        super.onCreate()
        self = this
        println("vpn on create")
    }

    override fun onDestroy() {
        println("vpn on destroy")
        // Only this instance may clear the static state; a newer service
        // instance may already own it.
        if (self === this) {
            disconnect()
            self = null
        }
        super.onDestroy()
    }

    override fun onRevoke() {
        println("vpn on revoke")
        super.onRevoke()
        if (self === this) {
            disconnect()
            stopSelf()
        }
    }

    private fun disconnect() {
        if (self === this && interfaceEstablished) {
            if (this::vpnInterface.isInitialized) {
                vpnInterface.close()
            }
            clearStatus()
            // Emit the stop event only after the interface is closed and the
            // state is cleared, so listeners observe a fully torn-down VPN.
            val data = JSObject()
            stopRequestId?.let { data.put("requestId", it) }
            triggerCallback("vpn_service_stop", data)
        }
    }

    private fun clearStatus() {
        ipv4Addr = null
        ipv6Addr = null
        routes = emptyArray()
        dns = null
        interfaceEstablished = false
        applicationMode = null
        allowedApplications = emptyArray()
        disallowedApplications = emptyArray()
        effectivePolicy = null
        skippedPackages = null
    }

    private fun createVpnInterface(args: Bundle?): ParcelFileDescriptor {
        val builder = Builder()
            .setSession("TauriVpnService")
            .setBlocking(false)

        val mtu = args?.getInt(MTU) ?: 1500
        val ipv4Addr = args?.getString(IPV4_ADDR) ?: "10.126.126.1/24"
        val ipv6Addr = args?.getString(IPV6_ADDR)
        val dns: String? = args?.getString(DNS)
        val routes = args?.getStringArray(ROUTES) ?: emptyArray()
        // "include" captures only the selected apps; anything else (including
        // legacy requests without a mode) is the all-apps branch, which must
        // exclude the VPN app itself.
        val mode = args?.getString(APPLICATION_MODE) ?: AppRoutingPolicyStore.MODE_ALL
        val allowedInput = args?.getStringArray(ALLOWED_APPLICATIONS) ?: emptyArray()
        val disallowedInput = args?.getStringArray(DISALLOWED_APPLICATIONS) ?: emptyArray()

        println(
            "vpn create vpn interface. mtu: $mtu, ipv4Addr: $ipv4Addr, ipv6Addr: $ipv6Addr, dns:" +
                "$dns, routes: ${java.util.Arrays.toString(routes)}, mode: $mode," +
                "allowedApplications: ${java.util.Arrays.toString(allowedInput)}," +
                "disallowedApplications: ${java.util.Arrays.toString(disallowedInput)}",
        )

        val ipParts = ipv4Addr.split("/")
        if (ipParts.size != 2) throw VpnEstablishException("invalid_addr", "Invalid IP addr string")
        builder.addAddress(ipParts[0], ipParts[1].toInt())
        ipv6Addr?.let {
            val ipv6Parts = it.split("/")
            if (ipv6Parts.size != 2) throw VpnEstablishException("invalid_addr", "Invalid IPv6 addr string")
            builder.addAddress(ipv6Parts[0], ipv6Parts[1].toInt())
        }

        builder.setMtu(mtu)
        dns?.let { builder.addDnsServer(it) }

        for (route in routes) {
            val routeParts = route.split("/")
            if (routeParts.size != 2) throw VpnEstablishException("invalid_route", "Invalid route cidr string")
            builder.addRoute(routeParts[0], routeParts[1].toInt())
        }

        // Re-resolve the policy against the current install state to close the
        // uninstall race between the plugin pre-check and this establish.
        val resolution = if (mode == AppRoutingPolicyStore.MODE_INCLUDE) {
            val identities = AppCatalog.getInstalledAppIdentities(this)
            val resolved = AppRoutingResolver.resolveInclude(identities, AppCatalog.getSelfUid(this), allowedInput.toList())
            if (resolved.effective.packages.isEmpty()) {
                throw VpnEstablishException("no_effective_apps", "no effective applications selected")
            }
            val added = mutableListOf<String>()
            val skipped = resolved.skipped.toMutableList()
            for (pkg in resolved.effective.packages) {
                try {
                    builder.addAllowedApplication(pkg)
                    added.add(pkg)
                } catch (e: PackageManager.NameNotFoundException) {
                    skipped.add(SkippedPackage(pkg, "not_installed"))
                }
            }
            if (added.isEmpty()) {
                throw VpnEstablishException("no_effective_apps", "all selected packages vanished")
            }
            val addedSet = added.toSet()
            resolved.copy(
                effective = resolved.effective.copy(
                    packages = added.sorted(),
                    uids = identities.filter { it.packageName in addedSet }.map { it.uid }.distinct().sorted(),
                ),
                skipped = skipped.distinctBy { it.packageName },
            )
        } else {
            val disallowed = (disallowedInput.toList() + packageName).distinct()
            for (pkg in disallowed) {
                builder.addDisallowedApplication(pkg)
            }
            AppRoutingResolver.resolveAll()
        }

        val iface = builder.also {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                it.setMetered(false)
            }
        }
            .establish()
            ?: throw VpnEstablishException("establish_failed", "Failed to init VpnService")

        // Record what was actually applied, not what was requested.
        ipv4Addr.let { TauriVpnService.ipv4Addr = it }
        TauriVpnService.ipv6Addr = ipv6Addr
        TauriVpnService.routes = routes
        TauriVpnService.dns = dns
        applicationMode = mode
        allowedApplications = if (mode == AppRoutingPolicyStore.MODE_INCLUDE) {
            resolution.effective.packages.toTypedArray()
        } else {
            emptyArray()
        }
        disallowedApplications = if (mode == AppRoutingPolicyStore.MODE_ALL) {
            (disallowedInput.toList() + packageName).distinct().toTypedArray()
        } else {
            emptyArray()
        }
        effectivePolicy = JSObject.fromJSONObject(AppRoutingResolver.effectiveToJson(resolution.effective))
        skippedPackages = JSArray.from(AppRoutingResolver.skippedToJson(resolution.skipped))

        return iface
    }
}
