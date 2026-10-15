package dev.codex.aubridge;

import android.accessibilityservice.AccessibilityService;
import android.graphics.Rect;
import android.os.Build;
import android.util.SparseArray;
import android.view.accessibility.AccessibilityNodeInfo;
import android.view.accessibility.AccessibilityWindowInfo;

import org.json.JSONObject;

import java.util.ArrayList;
import java.util.List;

/**
 * Android adapter for {@link TreeDump}. Runs on a bridge worker and never takes the {@link Ui}
 * guard, so a slow dump cannot stall the semantic and act paths.
 */
final class TreeWindows {
    static final int CAPABILITY = 8;

    private TreeWindows() {}

    static JSONObject dump(AccessibilityService service) throws Exception {
        List<TreeDump.Win> out = new ArrayList<>();
        if (Build.VERSION.SDK_INT >= 30) {
            SparseArray<List<AccessibilityWindowInfo>> all = service.getWindowsOnAllDisplays();
            for (int i = 0; i < all.size(); i++) add(out, all.valueAt(i));
        } else add(out, service.getWindows());
        return TreeDump.dump(out);
    }

    private static void add(List<TreeDump.Win> out, List<AccessibilityWindowInfo> windows) {
        if (windows != null) for (AccessibilityWindowInfo w : windows) out.add(new Window(w));
    }

    private static final class Window implements TreeDump.Win {
        private final AccessibilityWindowInfo window;
        private final AccessibilityNodeInfo root;

        Window(AccessibilityWindowInfo window) {
            this.window = window;
            this.root = window.getRoot();
        }

        public int type() {
            return window.getType();
        }

        public int layer() {
            return window.getLayer();
        }

        public int display() {
            return Build.VERSION.SDK_INT >= 30 ? window.getDisplayId() : 0;
        }

        public String title() {
            CharSequence title = window.getTitle();
            return title == null ? "" : title.toString();
        }

        public String pkg() {
            if (root == null) return "";
            CharSequence name = root.getPackageName();
            return name == null ? "" : name.toString();
        }

        public boolean active() {
            return window.isActive();
        }

        public boolean focused() {
            return window.isFocused();
        }

        public void bounds(int[] out) {
            Rect r = new Rect();
            window.getBoundsInScreen(r);
            out[0] = r.left;
            out[1] = r.top;
            out[2] = r.right;
            out[3] = r.bottom;
        }

        public TreeDump.Src root() {
            return root == null ? null : new Node(root);
        }
    }

    private static final class Node implements TreeDump.Src {
        private final AccessibilityNodeInfo node;

        Node(AccessibilityNodeInfo node) {
            this.node = node;
        }

        public String className() {
            return node.getClassName() == null ? "" : node.getClassName().toString();
        }

        public CharSequence text() {
            return node.getText();
        }

        public CharSequence description() {
            return node.getContentDescription();
        }

        public String viewId() {
            return node.getViewIdResourceName();
        }

        public void bounds(int[] out) {
            Rect r = new Rect();
            node.getBoundsInScreen(r);
            out[0] = r.left;
            out[1] = r.top;
            out[2] = r.right;
            out[3] = r.bottom;
        }

        public int flags() {
            return (node.isClickable() ? 1 : 0)
                    | (node.isCheckable() ? 2 : 0)
                    | (node.isChecked() ? 4 : 0)
                    | (node.isEditable() ? 8 : 0)
                    | (node.isFocused() ? 16 : 0)
                    | (node.isScrollable() ? 32 : 0)
                    | (node.isEnabled() ? 64 : 0)
                    | (node.isSelected() ? 128 : 0);
        }

        public boolean password() {
            return node.isPassword();
        }

        public int inputType() {
            return node.getInputType();
        }

        public int childCount() {
            return node.getChildCount();
        }

        public TreeDump.Src child(int index) {
            AccessibilityNodeInfo c = node.getChild(index);
            return c == null ? null : new Node(c);
        }

        @SuppressWarnings("deprecation")
        public void release() {
            node.recycle();
        }
    }
}
