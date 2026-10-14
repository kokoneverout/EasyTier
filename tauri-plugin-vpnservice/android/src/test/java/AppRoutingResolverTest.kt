package com.plugin.vpnservice

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Rule-level tests for the per-app routing policy resolver. These cover the
 * builder-facing decisions (allowed vs disallowed, empty-list refusal, self
 * uid rejection, shared-uid expansion) without a real system VPN.
 */
class AppRoutingResolverTest {

    private val selfUid = 10234

    private fun identities(): List<AppIdentity> = listOf(
        AppIdentity("com.example.browser", 10010, enabled = true),
        AppIdentity("com.example.browser2", 10011, enabled = true),
        AppIdentity("com.example.shared.a", 10012, enabled = true),
        AppIdentity("com.example.shared.b", 10012, enabled = true),
        AppIdentity("com.example.disabled", 10013, enabled = false),
        AppIdentity("com.example.vpnself", selfUid, enabled = true),
    )

    @Test
    fun includeNormalListResolvesOnlyAllowedBranch() {
        val resolution = AppRoutingResolver.resolveInclude(
            identities(),
            selfUid,
            listOf("com.example.browser"),
        )
        assertEquals(listOf("com.example.browser"), resolution.effective.packages)
        assertEquals(listOf(10010), resolution.effective.uids)
        assertTrue(resolution.skipped.isEmpty())
        assertEquals("include", resolution.effective.mode)
    }

    @Test
    fun includeSharedUidExpandsGroupMembers() {
        val resolution = AppRoutingResolver.resolveInclude(
            identities(),
            selfUid,
            listOf("com.example.shared.a"),
        )
        assertEquals(
            listOf("com.example.shared.a", "com.example.shared.b"),
            resolution.effective.packages,
        )
        assertEquals(listOf(10012), resolution.effective.uids)
    }

    @Test
    fun includeEmptySelectionYieldsEmptyEffective() {
        val resolution = AppRoutingResolver.resolveInclude(identities(), selfUid, emptyList())
        assertTrue(resolution.effective.packages.isEmpty())
        assertTrue(resolution.effective.uids.isEmpty())
    }

    @Test
    fun includeAllInvalidSelectionYieldsEmptyEffective() {
        val resolution = AppRoutingResolver.resolveInclude(
            identities(),
            selfUid,
            listOf("com.gone.app", "com.example.disabled", "com.example.vpnself"),
        )
        assertTrue(resolution.effective.packages.isEmpty())
        assertEquals(
            listOf(
                SkippedPackage("com.example.disabled", "disabled"),
                SkippedPackage("com.example.vpnself", "vpn_self_uid"),
                SkippedPackage("com.gone.app", "not_installed"),
            ),
            resolution.skipped.sortedBy { it.packageName },
        )
    }

    @Test
    fun includePartiallyInvalidKeepsRemainingApps() {
        val resolution = AppRoutingResolver.resolveInclude(
            identities(),
            selfUid,
            listOf("com.example.browser", "com.gone.app"),
        )
        assertEquals(listOf("com.example.browser"), resolution.effective.packages)
        assertEquals(listOf(SkippedPackage("com.gone.app", "not_installed")), resolution.skipped)
        // The uninstalled selection is preserved in the saved policy.
        assertEquals(
            listOf("com.example.browser", "com.gone.app"),
            resolution.policy.packages,
        )
    }

    @Test
    fun includeSelfUidIsAlwaysRejected() {
        val resolution = AppRoutingResolver.resolveInclude(
            identities(),
            selfUid,
            listOf("com.example.vpnself"),
        )
        assertTrue(resolution.effective.packages.isEmpty())
        assertEquals(listOf(SkippedPackage("com.example.vpnself", "vpn_self_uid")), resolution.skipped)
    }

    @Test
    fun allModeUsesDisallowedBranchWithEmptyEffective() {
        val resolution = AppRoutingResolver.resolveAll()
        assertEquals("all", resolution.effective.mode)
        assertTrue(resolution.effective.packages.isEmpty())
        assertTrue(resolution.effective.uids.isEmpty())
        assertTrue(resolution.skipped.isEmpty())
    }

    @Test
    fun allModePreservesPreviousSelectionForSwitchingBack() {
        val resolution = AppRoutingResolver.resolveAll(listOf("com.gone.app", "com.example.browser"))
        assertEquals(listOf("com.example.browser", "com.gone.app"), resolution.policy.packages)
        assertTrue(resolution.effective.uids.isEmpty())
    }

    @Test
    fun policyNormalizationDedupesAndSorts() {
        val policy = AppRoutingPolicy.normalize(
            AppRoutingPolicyStore.SCHEMA_VERSION,
            "include",
            listOf("b.app", "a.app", "b.app", " ", ""),
        )
        assertEquals(listOf("a.app", "b.app"), policy.packages)
    }

    @Test(expected = AppRoutingPolicyException::class)
    fun invalidModeIsRejected() {
        AppRoutingPolicy.normalize(AppRoutingPolicyStore.SCHEMA_VERSION, "exclude", emptyList())
    }

    @Test(expected = AppRoutingPolicyException::class)
    fun unsupportedSchemaVersionIsRejected() {
        AppRoutingPolicy.normalize(99, "include", emptyList())
    }
}
