package dev.codex.aubridge;

import java.nio.charset.StandardCharsets;
import java.security.SecureRandom;
import java.util.Base64;
import java.util.Iterator;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.function.LongSupplier;

/**
 * Short-lived session credentials for one helper lifecycle.
 *
 * The bootstrap endpoint is already restricted to the ADB shell/root UIDs, so
 * it mints a fresh credential instead of returning a persistent app-file
 * bearer token. Credentials expire, are bounded in number, and keep bounded
 * nonce replay state. The command socket additionally checks the peer UID and
 * the per-connection request sequence.
 */
final class SessionAuth {
    static final long SESSION_TTL_MS = 60L * 60L * 1000L;
    // A long agentic run can open hundreds of short-lived helper connections
    // while two persistent contract/pipe sessions remain active. Keep the
    // table bounded, but large enough that active long-lived sessions are not
    // evicted by normal one-shot CLI churn.
    static final int MAX_SESSIONS = 512;
    static final int MAX_NONCES_PER_SESSION = 2_048;
    private static final int MIN_NONCE_BYTES = 12;
    private static final int MAX_NONCE_BYTES = 128;
    private static final int SESSION_BYTES = 32;

    private final SecureRandom random = new SecureRandom();
    private final LongSupplier clock;
    private final Map<String, Session> sessions = new LinkedHashMap<>(MAX_SESSIONS, 0.75f, true);

    SessionAuth() {
        this(System::currentTimeMillis);
    }

    SessionAuth(LongSupplier clock) {
        this.clock = clock;
    }

    synchronized String issueSession() {
        purgeExpired(clock.getAsLong());
        String credential;
        do {
            byte[] bytes = new byte[SESSION_BYTES];
            random.nextBytes(bytes);
            credential = Base64.getUrlEncoder().withoutPadding().encodeToString(bytes);
        } while (sessions.containsKey(credential));
        while (sessions.size() >= MAX_SESSIONS) {
            Iterator<String> keys = sessions.keySet().iterator();
            if (!keys.hasNext()) break;
            keys.next();
            keys.remove();
        }
        long now = clock.getAsLong();
        sessions.put(credential, new Session(now, now + SESSION_TTL_MS));
        return credential;
    }

    synchronized void authenticate(String presented, String nonce) throws AuthError {
        long now = clock.getAsLong();
        purgeExpired(now);
        Session session = sessions.get(presented);
        if (session == null || session.expiresAtMs <= now) {
            throw new AuthError("E_AUTH", "invalid or expired helper session");
        }
        byte[] nonceBytes = nonce.getBytes(StandardCharsets.UTF_8);
        if (nonceBytes.length < MIN_NONCE_BYTES || nonceBytes.length > MAX_NONCE_BYTES) {
            throw new AuthError("E_AUTH", "missing or invalid session nonce");
        }
        if (session.seenNonces.containsKey(nonce)) {
            throw new AuthError("E_AUTH", "replayed session nonce");
        }
        // Never evict replay history. Eviction would let somebody holding a
        // still-live credential replay an observed old frame after enough
        // legitimate connections. A long-lived host session must re-bootstrap
        // once its bounded nonce budget is exhausted instead.
        if (session.seenNonces.size() >= MAX_NONCES_PER_SESSION) {
            throw new AuthError("E_AUTH", "helper session nonce budget exhausted");
        }
        session.seenNonces.put(nonce, Boolean.TRUE);
    }

    synchronized int sessionCountForTests() {
        purgeExpired(clock.getAsLong());
        return sessions.size();
    }

    private void purgeExpired(long now) {
        Iterator<Map.Entry<String, Session>> entries = sessions.entrySet().iterator();
        while (entries.hasNext()) {
            if (entries.next().getValue().expiresAtMs <= now) entries.remove();
        }
    }

    private static final class Session {
        final long createdAtMs;
        final long expiresAtMs;
        final Map<String, Boolean> seenNonces = new LinkedHashMap<>();

        Session(long createdAtMs, long expiresAtMs) {
            this.createdAtMs = createdAtMs;
            this.expiresAtMs = expiresAtMs;
        }
    }

    static final class AuthError extends Exception {
        final String code;

        AuthError(String code, String message) {
            super(message);
            this.code = code;
        }
    }
}
