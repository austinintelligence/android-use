package dev.codex.aubridge;

import android.content.ContentProvider;
import android.content.ContentValues;
import android.database.Cursor;
import android.database.MatrixCursor;
import android.net.Uri;
import android.os.Binder;
import android.util.Base64;

/** Hands the helper key to the ADB shell user, and to no one else. */
public final class KeyProvider extends ContentProvider {
    @Override
    public boolean onCreate() {
        return true;
    }

    @Override
    public Cursor query(
            Uri uri, String[] projection, String selection, String[] args, String order) {
        if (!HelperKey.mayRead(Binder.getCallingUid())) return null;
        MatrixCursor cursor = new MatrixCursor(new String[] {"key"});
        cursor.addRow(new Object[] {Base64.encodeToString(HelperKey.key(), Base64.NO_WRAP)});
        return cursor;
    }

    @Override
    public String getType(Uri uri) {
        return null;
    }

    @Override
    public Uri insert(Uri uri, ContentValues values) {
        return null;
    }

    @Override
    public int delete(Uri uri, String selection, String[] args) {
        return 0;
    }

    @Override
    public int update(Uri uri, ContentValues values, String selection, String[] args) {
        return 0;
    }
}
