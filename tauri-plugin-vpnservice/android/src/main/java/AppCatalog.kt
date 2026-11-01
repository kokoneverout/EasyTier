package com.plugin.vpnservice

import android.content.Context
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.drawable.Drawable
import android.util.Base64
import java.io.ByteArrayOutputStream

/** A full catalog entry for the app routing page. */
data class InstalledApp(
    val packageName: String,
    val label: String,
    val uid: Int,
    val isSystem: Boolean,
    val hasLauncher: Boolean,
    val enabled: Boolean,
    val hasInternetPermission: Boolean,
    /** Other packages that share this app's uid (Android applies VPN rules per uid). */
    val sharedUidPackages: List<String>,
    val selectable: Boolean,
    val unavailableReason: String?, // disabled | vpn_self_uid | null
    val packageNameAtUpdateTime: Long,
) {
    val iconKey: String get() = "$packageName@$packageNameAtUpdateTime"
}

data class AppCatalogData(
    val selfPackageName: String,
    val selfUid: Int,
    val apps: List<InstalledApp>,
)

data class AppIconData(
    val packageName: String,
    val iconKey: String,
    val dataUrl: String,
)

/**
 * Enumerates the installed applications of the current Android user and
 * rasterizes launcher icons on demand. All methods block on PackageManager and
 * must be called off the main thread.
 */
object AppCatalog {

    fun getSelfUid(context: Context): Int = context.applicationInfo.uid

    /** Cheap identity listing used by policy resolution; no labels, no icons. */
    fun getInstalledAppIdentities(context: Context): List<AppIdentity> {
        val pm = context.packageManager
        val apps = pm.getInstalledApplications(0)
        return apps.map { AppIdentity(it.packageName, it.uid, it.enabled) }
    }

    fun getCatalog(context: Context): AppCatalogData {
        val pm = context.packageManager
        val selfPackageName = context.packageName
        val selfUid = getSelfUid(context)

        val launcherIntent = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER)
        val launchablePackages = pm.queryIntentActivities(launcherIntent, 0)
            .mapNotNull { it.activityInfo?.applicationInfo?.packageName }
            .toSet()

        val installed = pm.getInstalledApplications(0)
        val savedPackages = AppRoutingPolicyStore.load(context)?.packages?.toSet() ?: emptySet()
        val uidGroups = installed.groupBy { it.uid }
        val internetByPackage = HashMap<String, Boolean>(installed.size)
        for (appInfo in installed) {
            internetByPackage[appInfo.packageName] =
                pm.checkPermission(android.Manifest.permission.INTERNET, appInfo.packageName) == PackageManager.PERMISSION_GRANTED
        }

        val apps = ArrayList<InstalledApp>(installed.size)
        for (appInfo in installed) {
            val pkg = appInfo.packageName
            val uid = appInfo.uid
            val group = uidGroups[uid] ?: emptyList()
            val hasInternet = internetByPackage[pkg] == true
            // Catalog rule: internet-capable apps, plus members of a shared-uid
            // group that contains an internet-capable app (they share fate).
            val groupHasInternet = group.any { internetByPackage[it.packageName] == true }
            if (!hasInternet && !groupHasInternet && pkg !in savedPackages) {
                continue
            }
            val unavailableReason = when {
                uid == selfUid -> "vpn_self_uid"
                !appInfo.enabled -> "disabled"
                else -> null
            }
            val label = try {
                appInfo.loadLabel(pm).toString()
            } catch (e: Exception) {
                pkg
            }
            apps.add(
                InstalledApp(
                    packageName = pkg,
                    label = label,
                    uid = uid,
                    isSystem = (appInfo.flags and ApplicationInfo.FLAG_SYSTEM) != 0,
                    hasLauncher = launchablePackages.contains(pkg),
                    enabled = appInfo.enabled,
                    hasInternetPermission = hasInternet,
                    sharedUidPackages = group.map { it.packageName }.filter { it != pkg },
                    selectable = unavailableReason == null,
                    unavailableReason = unavailableReason,
                    packageNameAtUpdateTime = pm.getPackageInfo(pkg, 0).lastUpdateTime,
                ),
            )
        }

        apps.sortWith(compareBy({ it.label.lowercase() }, { it.packageName }))
        return AppCatalogData(selfPackageName, selfUid, apps)
    }

    /**
     * Rasterizes the system-chosen icon drawable into a PNG data URL without
     * any restyling (no background, mask, border or filter).
     */
    fun getAppIcon(context: Context, packageName: String): AppIconData? {
        val pm = context.packageManager
        val appInfo = try {
            pm.getApplicationInfo(packageName, 0)
        } catch (e: Exception) {
            return null
        }
        val drawable: Drawable = try {
            pm.getApplicationIcon(appInfo)
        } catch (e: Exception) {
            return null
        }

        // 48 css px rendered at device density, capped to avoid huge bitmaps.
        val density = context.resources.displayMetrics.density
        val sizePx = (48.0 * density).toInt().coerceIn(48, 256)
        val bitmap = Bitmap.createBitmap(sizePx, sizePx, Bitmap.Config.ARGB_8888)
        val canvas = Canvas(bitmap)
        drawable.setBounds(0, 0, sizePx, sizePx)
        drawable.draw(canvas)

        val output = ByteArrayOutputStream()
        bitmap.compress(Bitmap.CompressFormat.PNG, 100, output)
        bitmap.recycle()
        val dataUrl = "data:image/png;base64," + Base64.encodeToString(output.toByteArray(), Base64.NO_WRAP)
        return AppIconData(packageName = packageName, iconKey = "$packageName@${pm.getPackageInfo(packageName, 0).lastUpdateTime}", dataUrl = dataUrl)
    }
}
