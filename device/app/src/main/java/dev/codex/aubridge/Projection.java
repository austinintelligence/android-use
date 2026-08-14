package dev.codex.aubridge;

import android.content.Context;
import android.content.Intent;
import android.media.projection.MediaProjection;
import android.media.projection.MediaProjectionManager;

/** Process-local, user-granted screen-capture token. It is never persisted. */
final class Projection {
    private static final Object LOCK = new Object();
    private static int resultCode;
    private static Intent data;

    private Projection() {}

    static void set(int code, Intent token) {
        synchronized (LOCK) {
            resultCode = code;
            data = token == null ? null : new Intent(token);
        }
    }

    static boolean available() {
        synchronized (LOCK) {
            return data != null;
        }
    }

    static MediaProjection acquire(Context context) {
        synchronized (LOCK) {
            if (data == null) return null;
            MediaProjectionManager manager = context.getSystemService(MediaProjectionManager.class);
            return manager == null ? null : manager.getMediaProjection(resultCode, new Intent(data));
        }
    }

    static void clear() {
        synchronized (LOCK) {
            data = null;
            resultCode = 0;
        }
    }
}
