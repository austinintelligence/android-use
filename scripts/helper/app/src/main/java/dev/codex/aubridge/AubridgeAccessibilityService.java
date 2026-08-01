package dev.codex.aubridge;

import android.accessibilityservice.AccessibilityService;
import android.accessibilityservice.GestureDescription;
import android.graphics.Path;
import android.graphics.Rect;
import android.os.Bundle;
import android.view.accessibility.AccessibilityEvent;
import android.view.accessibility.AccessibilityNodeInfo;

import org.json.JSONArray;
import org.json.JSONObject;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicLong;

public final class AubridgeAccessibilityService extends AccessibilityService {
    private static volatile AubridgeAccessibilityService instance;
    private final AtomicLong generation = new AtomicLong(0L);
    // Accessibility callbacks run on the service thread. Do not hold the
    // callback monitor while asking the framework for a root or traversing
    // nodes: some Android framework paths deliver the next callback synchronously
    // and can otherwise deadlock the helper socket.
    private final Object snapshotLock = new Object();
    private final Map<Long, AccessibilityNodeInfo> handles = new HashMap<>();
    private final List<String> events = new ArrayList<>();
    private JSONObject cachedSnapshot;
    private boolean cachedExpanded;
    private boolean dirty = true;

    static AubridgeAccessibilityService current() {
        return instance;
    }

    @Override
    protected void onServiceConnected() {
        instance = this;
    }

    @Override
    public void onAccessibilityEvent(AccessibilityEvent event) {
        synchronized (this) {
            generation.incrementAndGet();
            dirty = true;
            events.add(event.getEventType() + "@" + event.getWindowId());
            if (events.size() > 32) {
                events.remove(0);
            }
        }
    }

    @Override
    public void onInterrupt() {
    }

    @Override
    public void onDestroy() {
        synchronized (snapshotLock) {
            recycleHandles();
        }
        if (instance == this) {
            instance = null;
        }
        super.onDestroy();
    }

    JSONObject handle(String operation, JSONObject args) throws Exception {
        switch (operation) {
            case "ui.snapshot":
            case "ui.snap":
            case "ui.watch":
                return snapshot(args);
            case "ui.find":
                return find(args);
            case "ui.tap":
                return action(args, AccessibilityNodeInfo.ACTION_CLICK, "tapped");
            case "ui.long":
                return action(args, AccessibilityNodeInfo.ACTION_LONG_CLICK, "long_clicked");
            case "ui.set":
                return setText(args);
            case "ui.scroll":
                return scroll(args);
            case "ui.wait":
                return waitFor(args, false);
            case "ui.assert":
                return waitFor(args, true);
            case "ui.proof":
                return proof(args);
            case "ui.global":
                return global(args);
            case "ui.gesture":
                return gesture(args);
            default:
                throw new BridgeServer.BridgeError("E_ARGS", "unknown UI operation " + operation);
        }
    }

    private JSONObject snapshot(JSONObject args) throws Exception {
        synchronized (snapshotLock) {
        boolean expanded = args.optString("args", "").contains("--expanded");
        boolean compact = args.optString("args", "").contains("--compact");
        boolean delta = args.optString("args", "").contains("--delta");
        boolean frontier = args.optString("args", "").contains("--frontier");
        JSONArray requestArgs = args.optJSONArray("args");
        if (requestArgs != null) {
            for (int index = 0; index < requestArgs.length(); index++) {
                String value = requestArgs.optString(index);
                if ("--expanded".equals(value)) {
                    expanded = true;
                } else if ("--compact".equals(value)) {
                    compact = true;
                } else if ("--delta".equals(value)) {
                    delta = true;
                } else if ("--frontier".equals(value)) {
                    frontier = true;
                }
            }
        }
        if (frontier && delta) {
            throw new BridgeServer.BridgeError("E_ARGS", "--frontier and --delta are separate evidence levels");
        }
        if (!dirty && cachedSnapshot != null && cachedExpanded == expanded) {
            if (delta) {
                return new JSONObject()
                        .put("v", 1)
                        .put("g", cachedSnapshot.getLong("generation"))
                        .put("same", true);
            }
            return frontier ? renderFrontier(cachedSnapshot) : renderSnapshot(cachedSnapshot, compact);
        }
        JSONObject previous = cachedSnapshot;
        boolean previousCompatible = previous != null && cachedExpanded == expanded;
        recycleHandles();
        long nextGeneration = generation.incrementAndGet();
        AccessibilityNodeInfo root = getRootInActiveWindow();
        if (root == null) {
            throw new BridgeServer.BridgeError("E_UI", "no active accessibility window");
        }
        JSONArray nodes = new JSONArray();
        JSONArray previousNodes = previousCompatible && previous != null
                ? previous.optJSONArray("nodes")
                : null;
        traverse(root, nextGeneration, nodes, 0, expanded ? 800 : 200, expanded, previousNodes);
        JSONObject reply = new JSONObject();
        reply.put("generation", nextGeneration);
        reply.put("nodes", nodes);
        reply.put("events", new JSONArray(events));
        cachedSnapshot = new JSONObject(reply.toString());
        cachedExpanded = expanded;
        dirty = false;
        if (delta && previousCompatible) {
            return renderDelta(previous, reply, compact);
        }
        return frontier ? renderFrontier(reply) : renderSnapshot(reply, compact);
        }
    }

    private JSONObject renderSnapshot(JSONObject full, boolean compact) throws Exception {
        if (!compact) {
            return new JSONObject(full.toString());
        }
        JSONArray source = full.getJSONArray("nodes");
        JSONArray nodes = new JSONArray();
        for (int index = 0; index < source.length(); index++) {
            JSONObject node = source.getJSONObject(index);
            int flags = 0;
            if (node.optBoolean("clickable")) flags |= 1;
            if (node.optBoolean("enabled")) flags |= 2;
            if (node.optBoolean("checked")) flags |= 4;
            if (node.optBoolean("scrollable")) flags |= 8;
            nodes.put(new JSONArray()
                    .put(node.getLong("id"))
                    .put(cap(node.optString("text")))
                    .put(cap(node.optString("description")))
                    .put(role(node.optString("class_name")))
                    .put(flags)
                    .put(node.optJSONArray("bounds")));
        }
        return new JSONObject()
                .put("v", 1)
                .put("g", full.getLong("generation"))
                .put("complete", source.length() < 200)
                .put("n", nodes);
    }

    /**
     * Return only the currently visible, decision-bearing frontier. The full
     * cached tree remains available to find/query operations; this evidence
     * level is intentionally lossy and is never used to resolve a handle.
     */
    private JSONObject renderFrontier(JSONObject full) throws Exception {
        JSONArray source = full.getJSONArray("nodes");
        JSONArray nodes = new JSONArray();
        for (int index = 0; index < source.length(); index++) {
            JSONObject node = source.getJSONObject(index);
            if (!frontierNode(node)) {
                continue;
            }
            nodes.put(compactNode(node));
        }
        return new JSONObject()
                .put("v", 1)
                .put("g", full.getLong("generation"))
                .put("frontier", true)
                .put("complete", source.length() < 200)
                .put("n", nodes);
    }

    private static boolean frontierNode(JSONObject node) throws Exception {
        JSONArray bounds = node.optJSONArray("bounds");
        boolean visible = bounds != null
                && bounds.length() >= 4
                && bounds.optInt(2) > 0
                && bounds.optInt(3) > 24
                && bounds.optInt(0) < 1280
                && bounds.optInt(1) < 752;
        if (!visible) {
            return false;
        }
        return node.optBoolean("clickable")
                || node.optBoolean("scrollable")
                || !node.optString("text").isEmpty()
                || !node.optString("description").isEmpty();
    }

    private JSONObject renderDelta(JSONObject previous, JSONObject current, boolean compact) throws Exception {
        if (!compact) {
            return renderSnapshot(current, false);
        }
        JSONArray oldNodes = previous.getJSONArray("nodes");
        JSONArray newNodes = current.getJSONArray("nodes");
        JSONArray changed = new JSONArray();
        JSONArray removed = new JSONArray();
        int shared = Math.min(oldNodes.length(), newNodes.length());
        for (int index = 0; index < shared; index++) {
            JSONObject before = oldNodes.getJSONObject(index);
            JSONObject after = newNodes.getJSONObject(index);
            if (!nodeEquivalent(before, after)) {
                changed.put(new JSONArray().put(index).put(compactNode(after)));
            }
        }
        for (int index = shared; index < newNodes.length(); index++) {
            changed.put(new JSONArray().put(index).put(compactNode(newNodes.getJSONObject(index))));
        }
        for (int index = newNodes.length(); index < oldNodes.length(); index++) {
            removed.put(index);
        }
        return new JSONObject()
                .put("v", 1)
                .put("base", previous.getLong("generation"))
                .put("g", current.getLong("generation"))
                .put("complete", newNodes.length() < 200)
                .put("d", changed)
                .put("r", removed);
    }

    private static String cap(String value) {
        return value.length() <= 160 ? value : value.substring(0, 160);
    }

    private static String role(String className) {
        int marker = className.lastIndexOf('.');
        String value = marker >= 0 ? className.substring(marker + 1) : className;
        if (value.endsWith("Layout")) return "layout";
        if (value.endsWith("TextView")) return "text";
        if (value.endsWith("EditText")) return "input";
        if (value.endsWith("Button")) return "button";
        if (value.endsWith("Switch")) return "switch";
        if (value.endsWith("ScrollView")) return "scroll";
        return value;
    }

    private void traverse(AccessibilityNodeInfo node, long currentGeneration, JSONArray nodes, int depth, int limit, boolean expanded, JSONArray previousNodes) throws Exception {
        if (node == null || nodes.length() >= limit || depth > 32) {
            return;
        }
        Rect bounds = new Rect();
        node.getBoundsInScreen(bounds);
        JSONObject item = new JSONObject();
        item.put("text", string(node.getText()));
        item.put("description", string(node.getContentDescription()));
        item.put("resource_id", string(node.getViewIdResourceName()));
        // These fields are part of the public selector grammar. They stay in
        // the compact snapshot as short strings so a selector's meaning never
        // changes depending on whether the caller requested --expanded.
        item.put("class_name", string(node.getClassName()));
        item.put("package_name", string(node.getPackageName()));
        item.put("clickable", node.isClickable());
        item.put("enabled", node.isEnabled());
        item.put("scrollable", node.isScrollable());
        item.put("checked", node.isChecked());
        item.put("bounds", new JSONArray().put(bounds.left).put(bounds.top).put(bounds.right).put(bounds.bottom));
        int index = nodes.length();
        long id = (currentGeneration << 20) | index;
        if (previousNodes != null && index < previousNodes.length()) {
            JSONObject previous = previousNodes.optJSONObject(index);
            if (previous != null && nodeEquivalent(previous, item)) {
                id = previous.optLong("id", id);
            }
        }
        item.put("id", id);
        handles.put(id, AccessibilityNodeInfo.obtain(node));
        nodes.put(item);
        for (int childIndex = 0; childIndex < node.getChildCount(); childIndex++) {
            traverse(node.getChild(childIndex), currentGeneration, nodes, depth + 1, limit, expanded, previousNodes);
        }
    }

    private static boolean nodeEquivalent(JSONObject left, JSONObject right) throws Exception {
        return left.optString("text").equals(right.optString("text"))
                && left.optString("description").equals(right.optString("description"))
                && left.optString("resource_id").equals(right.optString("resource_id"))
                && left.optString("class_name").equals(right.optString("class_name"))
                && left.optString("package_name").equals(right.optString("package_name"))
                && left.optBoolean("clickable") == right.optBoolean("clickable")
                && left.optBoolean("enabled") == right.optBoolean("enabled")
                && left.optBoolean("scrollable") == right.optBoolean("scrollable")
                && left.optBoolean("checked") == right.optBoolean("checked")
                && left.optJSONArray("bounds").toString().equals(right.optJSONArray("bounds").toString());
    }

    private JSONObject find(JSONObject args) throws Exception {
        synchronized (snapshotLock) {
        JSONArray values = args.optJSONArray("args");
        String selector = values == null ? "" : values.optString(0, "");
        boolean compact = hasFlag(values, "--compact");
        JSONObject snapshot = snapshot(new JSONObject());
        JSONArray nodes = snapshot.getJSONArray("nodes");
        int requested = occurrence(selector);
        int matched = 0;
        for (int index = 0; index < nodes.length(); index++) {
            JSONObject node = nodes.getJSONObject(index);
            if (matches(node, selector)) {
                if (matched++ == requested) {
                    Object resultNode = compact ? compactNode(node) : node;
                    return new JSONObject().put("node", resultNode).put("generation", snapshot.getLong("generation"));
                }
            }
        }
        throw new BridgeServer.BridgeError("E_UI", "selector did not match: " + selector);
        }
    }

    private static boolean hasFlag(JSONArray values, String flag) {
        if (values == null) return false;
        for (int index = 0; index < values.length(); index++) {
            if (flag.equals(values.optString(index))) return true;
        }
        return false;
    }

    private static JSONArray compactNode(JSONObject node) throws Exception {
        int flags = 0;
        if (node.optBoolean("clickable")) flags |= 1;
        if (node.optBoolean("enabled")) flags |= 2;
        if (node.optBoolean("checked")) flags |= 4;
        if (node.optBoolean("scrollable")) flags |= 8;
        return new JSONArray()
                .put(node.getLong("id"))
                .put(cap(node.optString("text")))
                .put(cap(node.optString("description")))
                .put(role(node.optString("class_name")))
                .put(flags)
                .put(node.optJSONArray("bounds"));
    }

    private JSONObject action(JSONObject args, int action, String key) throws Exception {
        synchronized (this) {
            AccessibilityNodeInfo node = resolve(args);
            boolean success = node.performAction(action);
            if (!success) {
                throw new BridgeServer.BridgeError("E_UI", "accessibility node rejected action");
            }
        }
        // Some Android firmware can deliver the resulting accessibility event just after the
        // action acknowledgement. Keep the settle window local to mutating
        // actions so a rapid semantic batch does not race the next lookup;
        // this is not a global batch pacing delay.
        Thread.sleep(80L);
        return new JSONObject().put(key, true);
    }

    private JSONObject setText(JSONObject args) throws Exception {
        JSONArray values = args.optJSONArray("args");
        if (values == null || values.length() < 2) {
            throw new BridgeServer.BridgeError("E_ARGS", "ui set HANDLE_OR_SELECTOR TEXT");
        }
        synchronized (this) {
            AccessibilityNodeInfo node = resolve(values.optString(0));
            Bundle bundle = new Bundle();
            bundle.putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, values.optString(1));
            if (!node.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, bundle)) {
                throw new BridgeServer.BridgeError("E_UI", "node rejected text action");
            }
        }
        Thread.sleep(80L);
        return new JSONObject().put("set", true);
    }

    private JSONObject scroll(JSONObject args) throws Exception {
        JSONArray values = args.optJSONArray("args");
        String direction = values == null ? "forward" : values.optString(1, "forward");
        int action = "backward".equals(direction) ? AccessibilityNodeInfo.ACTION_SCROLL_BACKWARD : AccessibilityNodeInfo.ACTION_SCROLL_FORWARD;
        synchronized (this) {
            AccessibilityNodeInfo node = resolve(values == null ? "" : values.optString(0));
            if (!node.performAction(action)) {
                throw new BridgeServer.BridgeError("E_UI", "node rejected scroll action");
            }
        }
        Thread.sleep(80L);
        return new JSONObject().put("scrolled", direction);
    }

    private JSONObject global(JSONObject args) throws Exception {
        JSONArray values = args.optJSONArray("args");
        String requested = values == null ? "" : values.optString(0, "").toLowerCase();
        int action;
        switch (requested) {
            case "back": action = GLOBAL_ACTION_BACK; break;
            case "home": action = GLOBAL_ACTION_HOME; break;
            case "recents": action = GLOBAL_ACTION_RECENTS; break;
            case "notifications": action = GLOBAL_ACTION_NOTIFICATIONS; break;
            case "quick": action = GLOBAL_ACTION_QUICK_SETTINGS; break;
            default: throw new BridgeServer.BridgeError("E_ARGS", "ui global back|home|recents|notifications|quick");
        }
        if (!performGlobalAction(action)) {
            throw new BridgeServer.BridgeError("E_UI", "global accessibility action was rejected");
        }
        return new JSONObject().put("global", requested);
    }

    private JSONObject gesture(JSONObject args) throws Exception {
        JSONArray values = args.optJSONArray("args");
        if (values == null || values.length() < 4) {
            throw new BridgeServer.BridgeError("E_ARGS", "ui gesture X1 Y1 X2 Y2 [MS]");
        }
        float x1 = (float) values.optDouble(0, Double.NaN);
        float y1 = (float) values.optDouble(1, Double.NaN);
        float x2 = (float) values.optDouble(2, Double.NaN);
        float y2 = (float) values.optDouble(3, Double.NaN);
        long duration = Math.max(1L, Math.min(10_000L, values.optLong(4, 120L)));
        if (Float.isNaN(x1) || Float.isNaN(y1) || Float.isNaN(x2) || Float.isNaN(y2)) {
            throw new BridgeServer.BridgeError("E_ARGS", "gesture coordinates must be numeric");
        }
        Path path = new Path();
        path.moveTo(x1, y1);
        path.lineTo(x2, y2);
        GestureDescription gesture = new GestureDescription.Builder()
                .addStroke(new GestureDescription.StrokeDescription(path, 0L, duration))
                .build();
        CountDownLatch latch = new CountDownLatch(1);
        boolean accepted = dispatchGesture(gesture, new GestureResultCallback() {
            @Override
            public void onCompleted(GestureDescription ignored) {
                latch.countDown();
            }

            @Override
            public void onCancelled(GestureDescription ignored) {
                latch.countDown();
            }
        }, null);
        if (!accepted || !latch.await(duration + 1_000L, TimeUnit.MILLISECONDS)) {
            throw new BridgeServer.BridgeError("E_UI", "accessibility gesture was rejected or timed out");
        }
        return new JSONObject().put("gesture", true);
    }

    private JSONObject waitFor(JSONObject args, boolean assertion) throws Exception {
        JSONArray values = args.optJSONArray("args");
        String selector = values == null ? "" : values.optString(0, "");
        int timeout = values == null ? 5_000 : values.optInt(1, 5_000);
        timeout = Math.max(1, Math.min(timeout, 30_000));
        // Let the accessibility callback publish the mutation before the
        // first snapshot. This is a proof-local event yield, not a batch-wide
        // pacing delay, and it avoids holding the service monitor while the
        // The current window is still completing a scroll or text mutation.
        Thread.sleep(100L);
        long deadline = System.currentTimeMillis() + timeout;
        while (System.currentTimeMillis() < deadline) {
            try {
                JSONObject found = find(new JSONObject().put("args", new JSONArray().put(selector)));
                return new JSONObject().put(assertion ? "asserted" : "matched", true).put("node", found.getJSONObject("node"));
            } catch (BridgeServer.BridgeError ignored) {
                Thread.sleep(80L);
            }
        }
        throw new BridgeServer.BridgeError(assertion ? "E_ASSERT" : "E_TIMEOUT", "selector did not appear: " + selector);
    }

    /**
     * One bounded helper transaction for the smallest proof experiment:
     * find exactly one node, act on its session handle, wait for the
     * postcondition, and assert it.  Keeping this inside the accessibility
     * service removes four host/socket round trips while preserving the same
     * stale-handle and ambiguity checks.
     */
    private JSONObject proof(JSONObject args) throws Exception {
        JSONArray values = args.optJSONArray("args");
        if (values == null || values.length() < 2) {
            throw new BridgeServer.BridgeError("E_ARGS", "ui proof SELECTOR POSTSELECTOR [TIMEOUT_MS]");
        }
        String selector = values.optString(0, "");
        String postselector = values.optString(1, "");
        int timeout = Math.max(1, Math.min(values.optInt(2, 5_000), 30_000));
        JSONObject found = find(new JSONObject().put("args", new JSONArray().put(selector)));
        JSONObject node = found.getJSONObject("node");
        try {
            find(new JSONObject().put("args", new JSONArray().put(selector + "#1")));
            throw new BridgeServer.BridgeError("E_AMBIGUOUS", "selector matched more than one node");
        } catch (BridgeServer.BridgeError error) {
            if (!"E_UI".equals(error.code)) {
                throw error;
            }
        }
        String handle = Long.toString(node.getLong("id"));
        action(new JSONObject().put("args", new JSONArray().put(handle)), AccessibilityNodeInfo.ACTION_CLICK, "tapped");
        // Let the target window publish its accessibility event before the
        // event-driven wait begins; this is a small proof-local readiness
        // guard, not a global semantic action delay.
        Thread.sleep(80L);
        waitFor(new JSONObject().put("args", new JSONArray().put(postselector).put(timeout)), false);
        waitFor(new JSONObject().put("args", new JSONArray().put(postselector).put(timeout)), true);
        return new JSONObject()
                .put("proof", "find.unique>tap>wait>assert")
                .put("node", node)
                .put("postcondition", postselector)
                .put("generation", found.getLong("generation"));
    }

    private AccessibilityNodeInfo resolve(JSONObject args) throws Exception {
        JSONArray values = args.optJSONArray("args");
        return resolve(values == null ? "" : values.optString(0));
    }

    private AccessibilityNodeInfo resolve(String value) throws Exception {
        synchronized (snapshotLock) {
            try {
                // An accessibility event invalidates the cached node map, but
                // the callback and the host socket can race. Refresh once
                // before resolving a numeric handle so a changed/removed
                // handle deterministically becomes E_STALE instead of
                // reaching performAction() and producing a generic E_UI.
                // Stable trees keep the existing fast path because dirty is
                // false and the cached handle is reused without a traversal.
                if (dirty || cachedSnapshot == null) {
                    snapshot(new JSONObject());
                }
                long id = Long.parseLong(value);
                AccessibilityNodeInfo node = handles.get(id);
                if (node == null) {
                    throw new BridgeServer.BridgeError("E_STALE", "stale node handle");
                }
                return node;
            } catch (NumberFormatException ignored) {
                JSONObject found = find(new JSONObject().put("args", new JSONArray().put(value)));
                return handles.get(found.getJSONObject("node").getLong("id"));
            }
        }
    }

    private static boolean matches(JSONObject node, String selector) {
        for (String term : splitEscaped(selectorBody(selector), ',')) {
            String trimmed = term.trim();
            int contains = firstUnescaped(trimmed, '~');
            int equals = firstUnescaped(trimmed, '=');
            int split = contains >= 0 ? contains : equals;
            if (split < 1) return false;
            boolean containsMatch = contains >= 0;
            String field = trimmed.substring(0, split).trim().toLowerCase();
            String value = unescape(trimmed.substring(split + 1).trim());
            if (!("text".equals(field) || "desc".equals(field) || "id".equals(field)
                    || "class".equals(field) || "pkg".equals(field) || "clickable".equals(field)
                    || "enabled".equals(field) || "scrollable".equals(field) || "checked".equals(field)
                    || "bounds".equals(field))) return false;
            if (("clickable".equals(field) || "enabled".equals(field) || "scrollable".equals(field)
                    || "checked".equals(field)) && (containsMatch || !("true".equals(value) || "false".equals(value)))) return false;
            if ("text".equals(field) && !stringMatches(node.optString("text"), value, containsMatch)) return false;
            if ("desc".equals(field) && !stringMatches(node.optString("description"), value, containsMatch)) return false;
            if ("id".equals(field) && !stringMatches(node.optString("resource_id"), value, containsMatch)) return false;
            if ("class".equals(field) && !stringMatches(node.optString("class_name"), value, containsMatch)) return false;
            if ("pkg".equals(field) && !stringMatches(node.optString("package_name"), value, containsMatch)) return false;
            if ("clickable".equals(field) && (containsMatch || node.optBoolean("clickable") != Boolean.parseBoolean(value))) return false;
            if ("enabled".equals(field) && (containsMatch || node.optBoolean("enabled") != Boolean.parseBoolean(value))) return false;
            if ("scrollable".equals(field) && (containsMatch || node.optBoolean("scrollable") != Boolean.parseBoolean(value))) return false;
            if ("checked".equals(field) && (containsMatch || node.optBoolean("checked") != Boolean.parseBoolean(value))) return false;
            if ("bounds".equals(field) && (containsMatch || !bounds(node).equals(value))) return false;
        }
        return true;
    }

    private static String bounds(JSONObject node) {
        JSONArray values = node.optJSONArray("bounds");
        if (values == null || values.length() != 4) return "";
        return values.optInt(0) + "," + values.optInt(1) + "," + values.optInt(2) + "," + values.optInt(3);
    }

    private static boolean stringMatches(String actual, String expected, boolean contains) {
        return contains ? actual.contains(expected) : actual.equals(expected);
    }

    private static String selectorBody(String selector) {
        int marker = lastUnescaped(selector, '#');
        if (marker < 0) return selector;
        String suffix = selector.substring(marker + 1);
        for (int index = 0; index < suffix.length(); index++) {
            if (!Character.isDigit(suffix.charAt(index))) return selector;
        }
        return marker == 0 ? selector : selector.substring(0, marker);
    }

    private static int occurrence(String selector) {
        int marker = lastUnescaped(selector, '#');
        if (marker < 0) return 0;
        try {
            return Integer.parseInt(selector.substring(marker + 1));
        } catch (Exception ignored) {
            return 0;
        }
    }

    private static int firstUnescaped(String value, char needle) {
        boolean escaped = false;
        for (int index = 0; index < value.length(); index++) {
            char current = value.charAt(index);
            if (escaped) {
                escaped = false;
            } else if (current == '\\') {
                escaped = true;
            } else if (current == needle) {
                return index;
            }
        }
        return -1;
    }

    private static int lastUnescaped(String value, char needle) {
        boolean escaped = false;
        int match = -1;
        for (int index = 0; index < value.length(); index++) {
            char current = value.charAt(index);
            if (escaped) {
                escaped = false;
            } else if (current == '\\') {
                escaped = true;
            } else if (current == needle) {
                match = index;
            }
        }
        return match;
    }

    private static java.util.List<String> splitEscaped(String value, char delimiter) {
        java.util.ArrayList<String> parts = new java.util.ArrayList<>();
        StringBuilder current = new StringBuilder();
        boolean escaped = false;
        for (int index = 0; index < value.length(); index++) {
            char character = value.charAt(index);
            if (escaped) {
                current.append('\\').append(character);
                escaped = false;
            } else if (character == '\\') {
                escaped = true;
            } else if (character == delimiter) {
                parts.add(current.toString());
                current.setLength(0);
            } else {
                current.append(character);
            }
        }
        if (escaped) current.append('\\');
        parts.add(current.toString());
        return parts;
    }

    private static String unescape(String value) {
        StringBuilder result = new StringBuilder();
        boolean escaped = false;
        for (int index = 0; index < value.length(); index++) {
            char current = value.charAt(index);
            if (escaped) {
                result.append(current);
                escaped = false;
            } else if (current == '\\') {
                escaped = true;
            } else {
                result.append(current);
            }
        }
        if (escaped) result.append('\\');
        return result.toString();
    }

    private static String string(CharSequence value) {
        return value == null ? "" : value.toString();
    }

    private void recycleHandles() {
        for (AccessibilityNodeInfo node : handles.values()) {
            node.recycle();
        }
        handles.clear();
    }
}
