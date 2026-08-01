package dev.codex.aubridge;

/**
 * Current foreground-service component.
 *
 * Some Android firmware can retain a stale ActivityManager service record after an APK update
 * changes the package UID. Keeping the implementation in BridgeService while
 * using a new component name lets the updated helper start cleanly without
 * clearing app data or the authenticated token.
 */
public final class AuBridgeService extends BridgeService {
}
