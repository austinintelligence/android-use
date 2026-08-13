package dev.codex.aubridge;

import static org.junit.Assert.assertArrayEquals;
import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import android.content.Context;
import android.util.Base64;

import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;

import org.json.JSONObject;
import org.junit.Test;
import org.junit.runner.RunWith;

import java.io.File;
import java.io.FileOutputStream;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;

@RunWith(AndroidJUnit4.class)
public final class PrivateArtifactsInstrumentedTest {
    @Test
    public void authenticatedArtifactProtocolReadsChunksThenDeletes() throws Exception {
        Context context = InstrumentationRegistry.getInstrumentation().getTargetContext();
        File file = new File(context.getFilesDir(), "media/instrumented-transfer.bin");
        assertTrue(file.getParentFile().mkdirs() || file.getParentFile().isDirectory());
        byte[] expected = "authenticated-private-artifact".getBytes(StandardCharsets.UTF_8);
        try (FileOutputStream output = new FileOutputStream(file)) {
            output.write(expected);
            output.getFD().sync();
        }

        JSONObject opened = PrivateArtifacts.handle(context, "artifact.open", new JSONObject()
                .put("file", "media/instrumented-transfer.bin"));
        assertEquals(expected.length, opened.getLong("total_bytes"));
        assertEquals(hex(MessageDigest.getInstance("SHA-256").digest(expected)), opened.getString("sha256"));
        String handle = opened.getString("handle");

        JSONObject first = PrivateArtifacts.handle(context, "artifact.read", new JSONObject()
                .put("handle", handle)
                .put("file", "media/instrumented-transfer.bin")
                .put("offset", 0)
                .put("length", 9));
        assertEquals(9, first.getInt("bytes"));
        assertFalse(first.getBoolean("eof"));
        byte[] prefix = Base64.decode(first.getString("data"), Base64.NO_WRAP);
        assertArrayEquals(java.util.Arrays.copyOfRange(expected, 0, 9), prefix);

        JSONObject second = PrivateArtifacts.handle(context, "artifact.read", new JSONObject()
                .put("handle", handle)
                .put("file", "media/instrumented-transfer.bin")
                .put("offset", first.getLong("next_offset"))
                .put("length", PrivateArtifacts.MAX_CHUNK));
        assertTrue(second.getBoolean("eof"));
        byte[] suffix = Base64.decode(second.getString("data"), Base64.NO_WRAP);
        assertArrayEquals(java.util.Arrays.copyOfRange(expected, 9, expected.length), suffix);

        JSONObject deleted = PrivateArtifacts.handle(context, "artifact.delete", new JSONObject()
                .put("handle", handle)
                .put("file", "media/instrumented-transfer.bin"));
        assertTrue(deleted.getBoolean("removed"));
        assertFalse(file.exists());
    }

    private static String hex(byte[] bytes) {
        StringBuilder value = new StringBuilder(bytes.length * 2);
        for (byte item : bytes) value.append(String.format("%02x", item & 0xff));
        return value.toString();
    }
}
