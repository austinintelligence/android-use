package dev.codex.aubridge;

import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.Map;

/** Per-helper token and nonce gate. Nonces are bounded to prevent replay-state growth. */
final class SessionAuth {
    private final byte[] token;
    private final Map<String, Boolean> seenNonces = Collections.synchronizedMap(
            new LinkedHashMap<String, Boolean>(512, 0.75f, true) {
                @Override
                protected boolean removeEldestEntry(Map.Entry<String, Boolean> entry) {
                    return size() > 512;
                }
            });

    SessionAuth(String token) {
        this.token = token.getBytes(StandardCharsets.UTF_8);
    }

    void authenticate(String presented, String nonce) throws AuthError {
        if (!MessageDigest.isEqual(token, presented.getBytes(StandardCharsets.UTF_8))) {
            throw new AuthError("E_AUTH", "invalid helper token");
        }
        if (nonce.length() < 12 || seenNonces.put(nonce, Boolean.TRUE) != null) {
            throw new AuthError("E_AUTH", "missing or replayed session nonce");
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
