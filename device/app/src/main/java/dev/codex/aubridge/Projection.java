package dev.codex.aubridge;

import android.content.Context;
import android.content.Intent;
import android.media.projection.MediaProjection;
import android.media.projection.MediaProjectionManager;
import android.os.Build;

/** Process-local, user-granted screen-capture token. It is never persisted. */
final class Projection {
    private static final Recording.Token<Intent> TOKEN = new Recording.Token<>();
    private static volatile int resultCode;

    private Projection() {}

    static void set(int code, Intent token) {
        resultCode = code;
        TOKEN.set(token == null ? null : new Intent(token));
    }

    static boolean available() {
        return TOKEN.present();
    }

    /** Consumes the grant on Android 14+, so a second recording asks for consent again. */
    static MediaProjection acquire(Context context) {
        Intent data = TOKEN.take(Build.VERSION.SDK_INT);
        if (data == null) return null;
        MediaProjectionManager manager = context.getSystemService(MediaProjectionManager.class);
        return manager == null ? null : manager.getMediaProjection(resultCode, data);
    }

    static void clear() {
        TOKEN.clear();
        resultCode = 0;
    }
}
