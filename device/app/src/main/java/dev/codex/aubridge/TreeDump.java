package dev.codex.aubridge;

import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

import java.nio.charset.StandardCharsets;

/**
 * Serializes every accessibility window into one bounded reply for tools and the 2.0 core. Plain
 * Java over small interfaces so JVM tests can use a fake tree; {@link TreeWindows} adapts Android.
 *
 * <p>Node flag bits: 1 clickable, 2 checkable, 4 checked, 8 editable, 16 focused, 32 scrollable, 64
 * enabled, 128 selected, 256 password (text omitted).
 */
final class TreeDump {
    static final int MAX_NODES = 2048;
    static final int MAX_BYTES = 900_000;
    static final int MAX_SEEN = 2 * MAX_NODES;
    static final int MAX_DEPTH = 48;
    static final long MAX_MILLIS = 3000;
    static final int MAX_TEXT = 512;
    private static final int NODE_OVERHEAD = 8;
    private static final int WINDOW_OVERHEAD = 2048;

    interface Src {
        String className();

        CharSequence text();

        CharSequence description();

        String viewId();

        /** Fills left, top, right, bottom. */
        void bounds(int[] out);

        /**
         * Flag bits 1 to 128; the password bit is derived from {@link #password} and the input
         * type.
         */
        int flags();

        boolean password();

        int inputType();

        int childCount();

        Src child(int index);

        default void release() {}
    }

    interface Win {
        int type();

        int layer();

        int display();

        String title();

        String pkg();

        boolean active();

        boolean focused();

        void bounds(int[] out);

        Src root();
    }

    private final long deadline;
    private int nodes, seen;
    private long bytes;
    private boolean truncated;

    private TreeDump(long millis) {
        deadline = System.nanoTime() + millis * 1_000_000;
    }

    static JSONObject dump(Iterable<? extends Win> windows) throws JSONException {
        return dump(windows, MAX_MILLIS);
    }

    /** Stops walking after {@code millis}; what was cut is reported as truncated. */
    static JSONObject dump(Iterable<? extends Win> windows, long millis) throws JSONException {
        TreeDump d = new TreeDump(millis);
        JSONArray out = new JSONArray();
        for (Win w : windows) {
            if (d.bytes + WINDOW_OVERHEAD > MAX_BYTES) {
                d.truncated = true;
                break;
            }
            d.bytes += WINDOW_OVERHEAD;
            out.put(d.window(w));
        }
        return new JSONObject()
                .put("truncated", d.truncated)
                .put("nodes", d.nodes)
                .put("seen", d.seen)
                .put("windows", out);
    }

    private JSONObject window(Win w) throws JSONException {
        JSONObject o =
                new JSONObject()
                        .put("type", w.type())
                        .put("layer", w.layer())
                        .put("display", w.display())
                        .put("bounds", box(w))
                        .put("title", clip(w.title(), 128))
                        .put("pkg", clip(w.pkg(), 255))
                        .put("active", w.active())
                        .put("focused", w.focused());
        Src root = w.root();
        if (root == null) return o;
        JSONObject tree = node(root, 0);
        if (tree != null) o.put("root", tree);
        return o;
    }

    private static JSONArray box(Win w) {
        int[] b = new int[4];
        w.bounds(b);
        return new JSONArray().put(b[0]).put(b[1]).put(b[2]).put(b[3]);
    }

    private JSONObject node(Src s, int depth) throws JSONException {
        try {
            if (nodes >= MAX_NODES || depth > MAX_DEPTH || late()) return skip(s);
            JSONObject o = own(s);
            long cost = o.toString().getBytes(StandardCharsets.UTF_8).length + NODE_OVERHEAD;
            if (bytes + cost > MAX_BYTES) return skip(s);
            bytes += cost;
            nodes++;
            seen++;
            JSONArray kids = new JSONArray();
            for (int i = 0, n = s.childCount(); i < n; i++) {
                Src c = s.child(i);
                if (c == null) continue;
                JSONObject k = node(c, depth + 1);
                if (k != null) kids.put(k);
            }
            if (kids.length() > 0) o.put("k", kids);
            return o;
        } finally {
            s.release();
        }
    }

    private boolean late() {
        return System.nanoTime() - deadline > 0;
    }

    /** Counts what is left out, up to {@link #MAX_SEEN}, and marks the reply truncated. */
    private JSONObject skip(Src s) {
        truncated = true;
        if (seen >= MAX_SEEN || late()) return null;
        seen++;
        for (int i = 0, n = s.childCount(); i < n && seen < MAX_SEEN; i++) {
            Src c = s.child(i);
            if (c == null) continue;
            try {
                skip(c);
            } finally {
                c.release();
            }
        }
        return null;
    }

    private static JSONObject own(Src s) throws JSONException {
        boolean password = s.password() || isPasswordType(s.inputType());
        JSONObject o = new JSONObject().put("c", clip(s.className(), 128));
        if (!password) put(o, "t", s.text());
        put(o, "d", s.description());
        put(o, "i", s.viewId());
        int[] b = new int[4];
        s.bounds(b);
        o.put("b", new JSONArray().put(b[0]).put(b[1]).put(b[2]).put(b[3]));
        return o.put("f", (s.flags() & 0xff) | (password ? 256 : 0));
    }

    /** Text, number, and web password variations of {@code android.text.InputType}. */
    static boolean isPasswordType(int inputType) {
        int cls = inputType & 0xf, variation = inputType & 0xff0;
        if (cls == 1) return variation == 0x80 || variation == 0x90 || variation == 0xe0;
        return cls == 2 && variation == 0x10;
    }

    private static void put(JSONObject o, String key, CharSequence value) throws JSONException {
        if (value != null && value.length() > 0) o.put(key, clip(value, MAX_TEXT));
    }

    private static String clip(CharSequence s, int max) {
        if (s == null) return "";
        return s.length() <= max ? s.toString() : s.subSequence(0, max).toString();
    }
}
