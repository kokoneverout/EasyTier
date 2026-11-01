package com.plugin.vpnservice

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Intent
import android.net.VpnService
import android.os.Build
import android.os.ParcelFileDescriptor
import android.os.Bundle
import android.content.pm.ServiceInfo
import androidx.core.app.NotificationCompat
import android.system.OsConstants.AF_INET6
import java.net.InetAddress
import java.util.Arrays

import android.content.pm.PackageManager
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject

class VpnEstablishException(val code: String, message: String) : Exception(message)

class TauriVpnService : VpnService() {
    companion object {
        @JvmField var triggerCallback: (String, JSObject) -> Unit = { _, _ -> }
        @JvmField var self: TauriVpnService? = null
        @JvmField var ipv4Addr: String? = null
        @JvmField var ipv4Addrs: Array<String> = emptyArray()
        @JvmField var routes: Array<String> = emptyArray()
        @JvmField var dns: String? = null

        const val IPV4_ADDR = "IPV4_ADDR"
        const val IPV4_ADDRS = "IPV4_ADDRS"
        const val ROUTES = "ROUTES"
        const val DNS = "DNS"
        @JvmField @Volatile var interfaceEstablished: Boolean = false
        @JvmField var applicationMode: String? = null
        @JvmField var effectivePolicy: JSObject? = null
        @JvmField var skippedPackages: JSArray? = null
        @JvmField var allowedApplications: Array<String> = emptyArray()
        @JvmField var disallowedApplications: Array<String> = emptyArray()
        const val APPLICATION_MODE = "APPLICATION_MODE"
        const val ALLOWED_APPLICATIONS = "ALLOWED_APPLICATIONS"
        const val REQUEST_ID = "REQUEST_ID"
        const val DISALLOWED_APPLICATIONS = "DISALLOWED_APPLICATIONS"
        const val MTU = "MTU"

        private const val NOTIFICATION_CHANNEL_ID = "easytier_vpn_channel"
        private const val NOTIFICATION_ID = 1356
    }

    private lateinit var vpnInterface: ParcelFileDescriptor

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        println("vpn on start command ${intent?.getExtras()} $intent")
        var args = intent?.getExtras()
        ipv4Addrs = getIpv4Addrs(args)
        // A sticky restart can have no Intent after process death. The VPN
        // owner must supply the active addresses before creating an interface.
        if (ipv4Addrs.isEmpty()) {
            stopSelf(startId)
            return START_NOT_STICKY
        }
        startVpnForegroundService()
        ipv4Addr = ipv4Addrs.firstOrNull()
        routes = args?.getStringArray(ROUTES) ?: emptyArray()
        dns = args?.getString(DNS)

        try {
            vpnInterface = createVpnInterface(args)
            interfaceEstablished = true
        } catch (error: Exception) {
            val data = JSObject()
            data.put("errorMsg", (error as? VpnEstablishException)?.code ?: "establish_failed")
            data.put("requestId", args?.getString(REQUEST_ID))
            triggerCallback("vpn_service_error", data)
            clearStatus()
            stopSelf(startId)
            return START_NOT_STICKY
        }
        println("vpn created ${vpnInterface.fd}")

        var event_data = JSObject()
        event_data.put("fd", vpnInterface.fd)
        event_data.put("requestId", args?.getString(REQUEST_ID))
        event_data.put("effectivePolicy", effectivePolicy)
        event_data.put("skippedPackages", skippedPackages)
        triggerCallback("vpn_service_start", event_data)
        EasyTierVpnTileService.requestStateUpdate(this)

        return START_STICKY
    }

    override fun onCreate() {
        super.onCreate()
        self = this
        println("vpn on create")
    }

    override fun onDestroy() {
        println("vpn on destroy")
        disconnect()
        setMainForegroundServiceEnabled(true)
        stopForeground(STOP_FOREGROUND_REMOVE)
        self = null
        EasyTierVpnTileService.requestStateUpdate(this)
        super.onDestroy()
    }

    override fun onRevoke() {
        println("vpn on revoke")
        disconnect()
        setMainForegroundServiceEnabled(true)
        stopForeground(STOP_FOREGROUND_REMOVE)
        self = null
        EasyTierVpnTileService.requestStateUpdate(this)
        super.onRevoke()
    }

    private fun disconnect() {
        if (interfaceEstablished && this::vpnInterface.isInitialized) {
            triggerCallback("vpn_service_stop", JSObject())
            vpnInterface.close()
        }
        clearStatus()
    }

    private fun clearStatus() {
        interfaceEstablished = false
        applicationMode = null
        effectivePolicy = null
        skippedPackages = null
        allowedApplications = emptyArray()
        disallowedApplications = emptyArray()
        ipv4Addr = null
        ipv4Addrs = emptyArray()
        routes = emptyArray()
        dns = null
    }

    private fun startVpnForegroundService() {
        createNotificationChannel()

        val launchIntent = packageManager.getLaunchIntentForPackage(packageName)?.apply {
            addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP)
        }
        val contentIntent = launchIntent?.let {
            PendingIntent.getActivity(
                this,
                0,
                it,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
        }
        val notification = NotificationCompat.Builder(this, NOTIFICATION_CHANNEL_ID)
            .setSmallIcon(android.R.drawable.ic_menu_manage)
            .setContentTitle("EasyTier VPN is running")
            .setContentText("VPN connection is active")
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
            .apply { contentIntent?.let(::setContentIntent) }
            .build()

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            startForeground(
                NOTIFICATION_ID,
                notification,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE,
            )
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }

        // A TUN-backed network is now protected by this foreground service, so the
        // no-TUN keepalive service is redundant and would show a second notification.
        setMainForegroundServiceEnabled(false)
    }

    private fun setMainForegroundServiceEnabled(enabled: Boolean) {
        val intent = Intent().setClassName(packageName, "$packageName.MainForegroundService")
        if (!enabled) {
            stopService(intent)
            return
        }

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            startForegroundService(intent)
        } else {
            startService(intent)
        }
    }

    private fun createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = NotificationChannel(
                NOTIFICATION_CHANNEL_ID,
                "EasyTier VPN",
                NotificationManager.IMPORTANCE_LOW,
            )
            getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
        }
    }

    private fun getIpv4Addrs(args: Bundle?): Array<String> {
        val ipv4Addrs = args
            ?.getStringArray(IPV4_ADDRS)
            ?.filter { it.isNotBlank() }
            ?.toTypedArray()
            ?: emptyArray()
        if (ipv4Addrs.isNotEmpty()) {
            return ipv4Addrs
        }

        return listOfNotNull(args?.getString(IPV4_ADDR)?.takeIf { it.isNotBlank() }).toTypedArray()
    }

    private fun createVpnInterface(args: Bundle?): ParcelFileDescriptor {
        var builder = Builder()
                .setSession("TauriVpnService")
                .setBlocking(false)
        
        var mtu = args?.getInt(MTU) ?: 1500
        var ipv4Addrs = getIpv4Addrs(args)
        var dns: String? = args?.getString(DNS)
        var routes = args?.getStringArray(ROUTES) ?: emptyArray()
        var disallowedApplications = args?.getStringArray(DISALLOWED_APPLICATIONS) ?: emptyArray()

        println("vpn create vpn interface. mtu: $mtu, ipv4Addrs: ${java.util.Arrays.toString(ipv4Addrs)}, dns:" +
            "$dns, routes: ${java.util.Arrays.toString(routes)}," +
            "disallowedApplications:  ${java.util.Arrays.toString(disallowedApplications)}")

        for (ipv4Addr in ipv4Addrs) {
            val ipParts = ipv4Addr.split("/")
            if (ipParts.size != 2) throw IllegalArgumentException("Invalid IP addr string")
            builder.addAddress(ipParts[0], ipParts[1].toInt())
        }
        builder.allowFamily(AF_INET6)

        builder.setMtu(mtu)
        dns?.let { builder.addDnsServer(it) }

        for (route in routes) {
            val ipParts = route.split("/")
            if (ipParts.size != 2) throw IllegalArgumentException("Invalid route cidr string")
            builder.addRoute(ipParts[0], ipParts[1].toInt())
        }
        
        val mode = args?.getString(APPLICATION_MODE) ?: AppRoutingPolicyStore.MODE_ALL
        val allowedInput = args?.getStringArray(ALLOWED_APPLICATIONS) ?: emptyArray()
        val disallowedInput = disallowedApplications
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
        ?: throw IllegalStateException("Failed to init VpnService")
        // Record what was actually applied, not what was requested.
        TauriVpnService.routes = routes
        TauriVpnService.dns = dns
        applicationMode = mode
        allowedApplications = if (mode == AppRoutingPolicyStore.MODE_INCLUDE) {
            resolution.effective.packages.toTypedArray()
        } else {
            emptyArray()
        }
        TauriVpnService.disallowedApplications = if (mode == AppRoutingPolicyStore.MODE_ALL) {
            (disallowedInput.toList() + packageName).distinct().toTypedArray()
        } else {
            emptyArray()
        }
        effectivePolicy = JSObject.fromJSONObject(AppRoutingResolver.effectiveToJson(resolution.effective))
        skippedPackages = JSArray.from(AppRoutingResolver.skippedToJson(resolution.skipped))


        return iface
    }
}
