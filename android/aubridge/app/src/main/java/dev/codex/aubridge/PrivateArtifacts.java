package dev.codex.aubridge;

import android.content.Context;
import android.util.Base64;

import org.json.JSONObject;

import java.io.File;
import java.io.FileInputStream;
import java.io.RandomAccessFile;
import java.security.MessageDigest;
import java.util.Comparator;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;

/** Authenticated, chunked transfer for helper-private finite media artifacts. */
final class PrivateArtifacts {
    static final int MAX_CHUNK = 256 * 1024;
    private static final int MAX_SNAPSHOTS = 64;
    private static final long SNAPSHOT_TTL_MS = 10L * 60L * 1000L;
    private static final Map<String, Snapshot> SNAPSHOTS = new ConcurrentHashMap<>();

    private PrivateArtifacts() {
    }

    static JSONObject handle(Context context, String operation, JSONObject args) throws Exception {
        String relative = args.optString("file", "");
        if ("artifact.open".equals(operation)) {
            File file = resolve(context, relative);
            if (!file.isFile()) {
                throw new BridgeServer.BridgeError("E_ARTIFACT", "private artifact is unavailable");
            }
            pruneSnapshots();
            Snapshot snapshot = new Snapshot(
                    UUID.randomUUID().toString().replace("-", ""),
                    relative,
                    file.length(),
                    file.lastModified(),
                    sha256(file),
                    System.currentTimeMillis());
            SNAPSHOTS.put(snapshot.handle, snapshot);
            return new JSONObject()
                    .put("handle", snapshot.handle)
                    .put("file", snapshot.relative)
                    .put("total_bytes", snapshot.bytes)
                    .put("sha256", snapshot.sha256);
        }
        String handle = args.optString("handle", "");
        Snapshot snapshot = handle.isEmpty() ? null : requireSnapshot(handle);
        if (snapshot != null) relative = snapshot.relative;
        File file = resolve(context, relative);
        if ("artifact.read".equals(operation)) {
            if (!file.isFile()) {
                throw new BridgeServer.BridgeError("E_ARTIFACT", "private artifact is unavailable");
            }
            if (snapshot != null && (file.length() != snapshot.bytes || file.lastModified() != snapshot.modified)) {
                SNAPSHOTS.remove(snapshot.handle);
                throw new BridgeServer.BridgeError("E_STALE", "private artifact changed after it was opened");
            }
            long offset = args.optLong("offset", -1L);
            int requested = args.optInt("length", MAX_CHUNK);
            if (offset < 0L || requested < 1 || requested > MAX_CHUNK) {
                throw new BridgeServer.BridgeError("E_ARGS", "invalid artifact read range");
            }
            long total = snapshot == null ? file.length() : snapshot.bytes;
            if (offset > total) {
                throw new BridgeServer.BridgeError("E_ARGS", "artifact offset exceeds file size");
            }
            int count = (int) Math.min((long) requested, total - offset);
            byte[] bytes = new byte[count];
            try (RandomAccessFile input = new RandomAccessFile(file, "r")) {
                input.seek(offset);
                input.readFully(bytes);
            }
            long next = offset + count;
            return new JSONObject()
                    .put("handle", snapshot == null ? JSONObject.NULL : snapshot.handle)
                    .put("file", relative)
                    .put("offset", offset)
                    .put("next_offset", next)
                    .put("bytes", count)
                    .put("total_bytes", total)
                    .put("eof", next == total)
                    .put("data", Base64.encodeToString(bytes, Base64.NO_WRAP));
        }
        if ("artifact.delete".equals(operation)) {
            if (snapshot != null) SNAPSHOTS.remove(snapshot.handle);
            boolean existed = file.exists();
            if (existed && (!file.isFile() || !file.delete())) {
                throw new BridgeServer.BridgeError("E_ARTIFACT", "private artifact cleanup failed");
            }
            return new JSONObject().put("file", relative).put("removed", existed);
        }
        throw new BridgeServer.BridgeError("E_ARGS", "unknown private artifact operation");
    }

    private static Snapshot requireSnapshot(String handle) throws Exception {
        if (!handle.matches("[0-9a-f]{32}")) {
            throw new BridgeServer.BridgeError("E_ARTIFACT", "invalid private artifact handle");
        }
        Snapshot snapshot = SNAPSHOTS.get(handle);
        if (snapshot == null || System.currentTimeMillis() - snapshot.created > SNAPSHOT_TTL_MS) {
            SNAPSHOTS.remove(handle);
            throw new BridgeServer.BridgeError("E_STALE", "private artifact handle expired");
        }
        return snapshot;
    }

    private static void pruneSnapshots() {
        long now = System.currentTimeMillis();
        SNAPSHOTS.values().removeIf(snapshot -> now - snapshot.created > SNAPSHOT_TTL_MS);
        while (SNAPSHOTS.size() >= MAX_SNAPSHOTS) {
            Snapshot oldest = SNAPSHOTS.values().stream()
                    .min(Comparator.comparingLong(snapshot -> snapshot.created))
                    .orElse(null);
            if (oldest == null) break;
            SNAPSHOTS.remove(oldest.handle, oldest);
        }
    }

    private static String sha256(File file) throws Exception {
        MessageDigest digest = MessageDigest.getInstance("SHA-256");
        byte[] buffer = new byte[64 * 1024];
        try (FileInputStream input = new FileInputStream(file)) {
            int count;
            while ((count = input.read(buffer)) != -1) digest.update(buffer, 0, count);
        }
        StringBuilder encoded = new StringBuilder(64);
        for (byte value : digest.digest()) encoded.append(String.format("%02x", value & 0xff));
        return encoded.toString();
    }

    private static final class Snapshot {
        final String handle;
        final String relative;
        final long bytes;
        final long modified;
        final String sha256;
        final long created;

        Snapshot(String handle, String relative, long bytes, long modified, String sha256, long created) {
            this.handle = handle;
            this.relative = relative;
            this.bytes = bytes;
            this.modified = modified;
            this.sha256 = sha256;
            this.created = created;
        }
    }

    static boolean isValidRelativePath(String relative) {
        return relative != null
                && relative.matches("media/[A-Za-z0-9._-]{1,128}")
                && !relative.contains("..");
    }

    private static File resolve(Context context, String relative) throws Exception {
        if (!isValidRelativePath(relative)) {
            throw new BridgeServer.BridgeError("E_ARTIFACT", "invalid private artifact path");
        }
        File media = new File(context.getFilesDir(), "media").getCanonicalFile();
        File file = new File(context.getFilesDir(), relative).getCanonicalFile();
        if (!media.equals(file.getParentFile())) {
            throw new BridgeServer.BridgeError("E_ARTIFACT", "private artifact escaped media root");
        }
        return file;
    }
}
