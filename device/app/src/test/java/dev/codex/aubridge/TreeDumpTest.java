package dev.codex.aubridge;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertTrue;

import org.json.JSONArray;
import org.json.JSONObject;
import org.junit.Test;

import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collections;
import java.util.List;

public final class TreeDumpTest {
    private static final class Fake implements TreeDump.Src {
        String cls = "android.view.View", text, desc, id;
        int flags, inputType, left, top, right, bottom;
        boolean password;
        final List<Fake> kids = new ArrayList<>();

        Fake(String cls, String text, int flags) {
            this.cls = cls;
            this.text = text;
            this.flags = flags;
        }

        Fake box(int l, int t, int r, int b) {
            left = l;
            top = t;
            right = r;
            bottom = b;
            return this;
        }

        Fake add(Fake kid) {
            kids.add(kid);
            return this;
        }

        public String className() {
            return cls;
        }

        public CharSequence text() {
            return text;
        }

        public CharSequence description() {
            return desc;
        }

        public String viewId() {
            return id;
        }

        public void bounds(int[] out) {
            out[0] = left;
            out[1] = top;
            out[2] = right;
            out[3] = bottom;
        }

        public int flags() {
            return flags;
        }

        public boolean password() {
            return password;
        }

        public int inputType() {
            return inputType;
        }

        public int childCount() {
            return kids.size();
        }

        public TreeDump.Src child(int index) {
            return kids.get(index);
        }
    }

    private static final class Window implements TreeDump.Win {
        final int type, layer;
        final String title, pkg;
        final boolean active;
        final int[] box;
        final Fake root;

        Window(int type, int layer, String title, String pkg, boolean active, Fake root) {
            this.type = type;
            this.layer = layer;
            this.title = title;
            this.pkg = pkg;
            this.active = active;
            this.root = root;
            this.box = new int[] {root.left, root.top, root.right, root.bottom};
        }

        public int type() {
            return type;
        }

        public int layer() {
            return layer;
        }

        public int display() {
            return 0;
        }

        public String title() {
            return title;
        }

        public String pkg() {
            return pkg;
        }

        public boolean active() {
            return active;
        }

        public boolean focused() {
            return active;
        }

        public void bounds(int[] out) {
            System.arraycopy(box, 0, out, 0, 4);
        }

        public TreeDump.Src root() {
            return root;
        }
    }

    private static JSONObject goldenTree() throws Exception {
        for (String candidate :
                new String[] {
                    "protocol-golden.json", "../protocol-golden.json", "../../protocol-golden.json"
                }) {
            if (Files.isRegularFile(Paths.get(candidate)))
                return new JSONObject(
                                new String(
                                        Files.readAllBytes(Paths.get(candidate)),
                                        StandardCharsets.UTF_8))
                        .getJSONArray("tree")
                        .getJSONObject(2);
        }
        throw new java.io.FileNotFoundException("protocol-golden.json");
    }

    /** Structural equality; android.jar's org.json has no similar() and orders keys differently. */
    private static void assertSame(Object expected, Object actual) throws Exception {
        if (expected instanceof JSONObject && actual instanceof JSONObject) {
            JSONObject a = (JSONObject) expected, b = (JSONObject) actual;
            assertEquals(a.length(), b.length());
            for (java.util.Iterator<String> keys = a.keys(); keys.hasNext(); ) {
                String key = keys.next();
                assertTrue("missing " + key, b.has(key));
                assertSame(a.get(key), b.get(key));
            }
        } else if (expected instanceof JSONArray && actual instanceof JSONArray) {
            JSONArray a = (JSONArray) expected, b = (JSONArray) actual;
            assertEquals(a.length(), b.length());
            for (int i = 0; i < a.length(); i++) assertSame(a.get(i), b.get(i));
        } else if (expected instanceof Number && actual instanceof Number) {
            assertEquals(((Number) expected).longValue(), ((Number) actual).longValue());
        } else {
            assertEquals(expected, actual);
        }
    }

    private static List<Window> signIn() {
        Fake name = new Fake("android.widget.EditText", "Ada", 8 | 16 | 64).box(48, 300, 1032, 420);
        name.id = "com.example.app:id/name";
        // Password found by its input type (text password), not by the platform flag.
        Fake secret =
                new Fake("android.widget.EditText", "hunter2", 8 | 64).box(48, 440, 1032, 560);
        secret.desc = "Password";
        secret.inputType = 0x81;
        Fake go = new Fake("android.widget.Button", "Continue", 1 | 64).box(48, 600, 1032, 720);
        Fake app =
                new Fake("android.widget.FrameLayout", null, 64)
                        .box(0, 0, 1080, 2400)
                        .add(name)
                        .add(secret)
                        .add(go);
        Fake bar = new Fake("android.widget.FrameLayout", "", 64).box(0, 0, 1080, 96);
        return Arrays.asList(
                new Window(1, 0, "Sign in", "com.example.app", true, app),
                new Window(3, 1, "", "com.android.systemui", false, bar));
    }

    @Test
    public void replyMatchesTheGoldenTheHostParses() throws Exception {
        JSONObject reply = TreeDump.dump(signIn());
        assertSame(goldenTree(), reply);
    }

    @Test
    public void passwordTextIsOmittedByFlagAndByInputType() throws Exception {
        Fake flagged = new Fake("android.widget.EditText", "hunter2", 8);
        flagged.password = true;
        Fake numeric = new Fake("android.widget.EditText", "1234", 8);
        numeric.inputType = 0x12;
        Fake visible = new Fake("android.widget.EditText", "hunter2", 8);
        visible.inputType = 0x91;
        Fake plain = new Fake("android.widget.EditText", "Ada", 8);
        plain.inputType = 0x1;
        Fake root = new Fake("android.view.View", null, 0);
        for (Fake f : new Fake[] {flagged, numeric, visible, plain}) root.add(f);
        JSONObject reply =
                TreeDump.dump(Collections.singletonList(new Window(1, 0, "", "pkg", true, root)));
        JSONArray kids =
                reply.getJSONArray("windows")
                        .getJSONObject(0)
                        .getJSONObject("root")
                        .getJSONArray("k");
        for (int i = 0; i < 3; i++) {
            JSONObject node = kids.getJSONObject(i);
            assertFalse("node " + i, node.has("t"));
            assertEquals(256, node.getInt("f") & 256);
        }
        assertEquals("Ada", kids.getJSONObject(3).getString("t"));
        assertFalse(reply.toString().contains("hunter2"));
        assertFalse(reply.toString().contains("1234"));
    }

    @Test
    public void nodeCapTruncatesAndSaysSo() throws Exception {
        Fake root = new Fake("android.view.View", null, 0);
        for (int i = 0; i < 3000; i++) root.add(new Fake("android.view.View", "n" + i, 0));
        JSONObject reply =
                TreeDump.dump(Collections.singletonList(new Window(1, 0, "", "pkg", true, root)));
        assertTrue(reply.getBoolean("truncated"));
        assertEquals(TreeDump.MAX_NODES, reply.getInt("nodes"));
        assertEquals(3001, reply.getInt("seen"));
    }

    @Test
    public void byteCapTruncatesBelowTheFrameLimit() throws Exception {
        String wide = String.join("", Collections.nCopies(TreeDump.MAX_TEXT, "é"));
        Fake root = new Fake("android.view.View", null, 0);
        for (int i = 0; i < 1500; i++) root.add(new Fake("android.view.View", wide, 0));
        JSONObject reply =
                TreeDump.dump(Collections.singletonList(new Window(1, 0, "", "pkg", true, root)));
        assertTrue(reply.getBoolean("truncated"));
        assertTrue(reply.getInt("nodes") < 1500);
        int size = reply.toString().getBytes(StandardCharsets.UTF_8).length;
        assertTrue("size " + size, size <= TreeDump.MAX_BYTES);
        assertTrue(size < Bridge.MAX_FRAME);
    }

    @Test
    public void smallTreesAreNotTruncatedAndDepthIsBounded() throws Exception {
        assertFalse(TreeDump.dump(signIn()).getBoolean("truncated"));
        Fake top = new Fake("android.view.View", null, 0), tip = top;
        for (int i = 0; i < 200; i++) {
            Fake next = new Fake("android.view.View", null, 0);
            tip.add(next);
            tip = next;
        }
        JSONObject deep =
                TreeDump.dump(Collections.singletonList(new Window(1, 0, "", "pkg", true, top)));
        assertTrue(deep.getBoolean("truncated"));
        assertEquals(TreeDump.MAX_DEPTH + 1, deep.getInt("nodes"));
        assertNotNull(deep.getJSONArray("windows").getJSONObject(0).optJSONObject("root"));
    }

    @Test
    public void aSpentTimeBudgetTruncates() throws Exception {
        Fake root =
                new Fake("android.view.View", null, 0).add(new Fake("android.view.View", "a", 0));
        JSONObject reply =
                TreeDump.dump(
                        Collections.singletonList(new Window(1, 0, "", "pkg", true, root)), -1);
        assertTrue(reply.getBoolean("truncated"));
        assertEquals(0, reply.getInt("nodes"));
    }

    @Test
    public void capabilityBitIsNewAndOutsideTheExistingMask() {
        assertEquals(8, TreeWindows.CAPABILITY);
        assertEquals(0, TreeWindows.CAPABILITY & 7);
    }
}
