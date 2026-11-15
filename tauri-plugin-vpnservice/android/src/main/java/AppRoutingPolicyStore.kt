package com.plugin.vpnservice

import android.content.Context
import org.json.JSONArray
import org.json.JSONObject

/**
 * Persisted per-app routing preference for this device.
 *
 * Modes:
 *  - "include": only the selected packages may use the VPN (Android allow list).
 *  - "all": every application may use the VPN; the VPN app itself stays excluded.
 *
 * The store is the single source of truth for the saved policy. The web layer
 * only keeps an editing draft; nothing else may persist a second copy.
 */
data class AppRoutingPolicy(
    val schemaVersion: Int,
    val mode: String,
    val packages: List<String>,
) {
    init {
        if (mode != AppRoutingPolicyStore.MODE_INCLUDE && mode != AppRoutingPolicyStore.MODE_ALL) {
            throw AppRoutingPolicyException("invalid app routing mode: $mode")
        }
        if (schemaVersion != AppRoutingPolicyStore.SCHEMA_VERSION) {
            throw AppRoutingPolicyException("unsupported app routing schemaVersion: $schemaVersion")
        }
    }

    fun toJson(): JSONObject = JSONObject()
        .put("schemaVersion", schemaVersion)
        .put("mode", mode)
        .put("packages", JSONArray(packages))

    companion object {
        fun normalize(schemaVersion: Int, mode: String, packages: List<String>): AppRoutingPolicy =
            AppRoutingPolicy(
                schemaVersion = schemaVersion,
                mode = mode,
                packages = packages
                    .map { it.trim() }
                    .filter { it.isNotEmpty() }
                    .distinct()
                    .sorted(),
            )
    }
}

class AppRoutingPolicyException(message: String) : Exception(message)

object AppRoutingPolicyStore {
    const val SCHEMA_VERSION = 1
    const val MODE_INCLUDE = "include"
    const val MODE_ALL = "all"

    private const val PREFS_NAME = "easytier_app_routing"
    private const val KEY_POLICY = "policy_v1"

    private fun prefs(context: Context) =
        context.applicationContext.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)

    /**
     * Returns null when no policy has ever been saved (fresh install).
     * Throws AppRoutingPolicyException when a policy exists but cannot be read;
     * such a record must not be silently treated as "absent".
     */
    fun load(context: Context): AppRoutingPolicy? {
        val raw = prefs(context).getString(KEY_POLICY, null) ?: return null
        val obj = try {
            JSONObject(raw)
        } catch (e: Exception) {
            throw AppRoutingPolicyException("saved app routing policy is not valid JSON: ${e.message}")
        }
        return fromJson(obj)
    }

    fun save(context: Context, policy: AppRoutingPolicy) {
        if (!prefs(context).edit().putString(KEY_POLICY, policy.toJson().toString()).commit()) {
            throw AppRoutingPolicyException("policy_save_failed")
        }
    }

    fun fromJson(obj: JSONObject): AppRoutingPolicy {
        val version = obj.optInt("schemaVersion", -1)
        if (version != SCHEMA_VERSION) {
            throw AppRoutingPolicyException("unsupported app routing schemaVersion: $version")
        }
        val mode = obj.optString("mode")
        if (mode != MODE_INCLUDE && mode != MODE_ALL) {
            throw AppRoutingPolicyException("invalid app routing mode: $mode")
        }
        val arr = obj.optJSONArray("packages") ?: throw AppRoutingPolicyException("app routing packages must be an array")
        val packages = (0 until arr.length()).map { idx ->
            val pkg = arr.optString(idx)
            if (pkg.isEmpty()) throw AppRoutingPolicyException("app routing package name must not be empty")
            pkg
        }
        return AppRoutingPolicy.normalize(SCHEMA_VERSION, mode, packages)
    }
}

/** Minimal identity of an installed package; enough for policy resolution. */
data class AppIdentity(
    val packageName: String,
    val uid: Int,
    val enabled: Boolean,
)

data class SkippedPackage(
    val packageName: String,
    val reason: String, // not_installed | disabled | vpn_self_uid
)

data class EffectiveAppRouting(
    val mode: String,
    /** Installed + enabled packages that the VPN builder will capture, shared-UID groups expanded. */
    val packages: List<String>,
    /** Distinct uids covered by the packages, sorted. Runtime only; never persisted. */
    val uids: List<Int>,
)

data class AppRoutingResolution(
    /** Normalized saved selection; keeps uninstalled package names for later cleanup. */
    val policy: AppRoutingPolicy,
    val effective: EffectiveAppRouting,
    val skipped: List<SkippedPackage>,
)

/**
 * Pure resolver shared by get/set policy commands and VPN start so that the
 * three call sites never disagree about what a saved policy means.
 */
object AppRoutingResolver {

    fun resolveAll(packages: List<String> = emptyList()): AppRoutingResolution = AppRoutingResolution(
        policy = AppRoutingPolicy.normalize(
            AppRoutingPolicyStore.SCHEMA_VERSION,
            AppRoutingPolicyStore.MODE_ALL,
            packages,
        ),
        effective = EffectiveAppRouting(AppRoutingPolicyStore.MODE_ALL, emptyList(), emptyList()),
        skipped = emptyList(),
    )

    fun resolveInclude(
        identities: List<AppIdentity>,
        selfUid: Int,
        packages: List<String>,
    ): AppRoutingResolution {
        val byPackage = identities.associateBy { it.packageName }
        val byUid = identities.groupBy { it.uid }

        val skipped = mutableListOf<SkippedPackage>()
        val effectivePackages = sortedSetOf<String>()
        val effectiveUids = sortedSetOf<Int>()

        val normalized = AppRoutingPolicy.normalize(
            AppRoutingPolicyStore.SCHEMA_VERSION,
            AppRoutingPolicyStore.MODE_INCLUDE,
            packages,
        )
        for (pkg in normalized.packages) {
            val identity = byPackage[pkg]
            when {
                identity == null -> skipped.add(SkippedPackage(pkg, "not_installed"))
                identity.uid == selfUid -> skipped.add(SkippedPackage(pkg, "vpn_self_uid"))
                !identity.enabled -> skipped.add(SkippedPackage(pkg, "disabled"))
                else -> {
                    effectiveUids.add(identity.uid)
                    val group = byUid[identity.uid] ?: listOf(identity)
                    for (member in group) {
                        effectivePackages.add(member.packageName)
                    }
                }
            }
        }

        return AppRoutingResolution(
            policy = normalized,
            effective = EffectiveAppRouting(AppRoutingPolicyStore.MODE_INCLUDE, effectivePackages.toList(), effectiveUids.toList()),
            skipped = skipped,
        )
    }

    fun skippedToJson(skipped: List<SkippedPackage>): JSONArray {
        val arr = JSONArray()
        for (item in skipped) {
            arr.put(JSONObject().put("packageName", item.packageName).put("reason", item.reason))
        }
        return arr
    }

    fun effectiveToJson(effective: EffectiveAppRouting): JSONObject = JSONObject()
        .put("mode", effective.mode)
        .put("packages", JSONArray(effective.packages))
        .put("uids", JSONArray(effective.uids))
}
