package dev.codex.aubridge;

import android.accessibilityservice.GestureDescription;
import android.content.Context;
import android.content.Intent;
import android.graphics.Path;
import android.graphics.Rect;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.SystemClock;
import android.provider.Settings;
import android.view.accessibility.AccessibilityNodeInfo;

import org.json.JSONArray;
import org.json.JSONException;

import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;

final class Vm {
    private final Context context;
    private final Capture capture;
    private final Map<String, R> done =
            new LinkedHashMap<String, R>(128, .75f, true) {
                @Override
                protected boolean removeEldestEntry(Map.Entry<String, R> e) {
                    return size() > 128;
                }
            };
    private final Set<String> running = new HashSet<>();

    Vm(Context context, Capture capture) {
        this.context = context;
        this.capture = capture;
    }

    JSONArray run(long seq, JSONArray q) {
        String id = q.optString(3, "");
        synchronized (done) {
            R prior = done.get(id);
            if (prior != null) return prior.wire(seq);
            if (running.contains(id)) return new R(7, 0, 0, 0, null).wire(seq);
            running.add(id);
        }
        R result;
        try {
            result = execute(q);
        } catch (Exception e) {
            result = new R(failure(e), gen(), 0, 0, null);
        } finally {
            synchronized (done) {
                running.remove(id);
            }
        }
        synchronized (done) {
            done.put(id, result);
        }
        return result.wire(seq);
    }

    private R execute(JSONArray q) throws Exception {
        if (q.length() != 7) throw new Bridge.Limit();
        long expected = q.getLong(2);
        String id = q.getString(3);
        if (!id.matches("[A-Za-z0-9_-]{1,64}")) throw new Bridge.Limit();
        int deadline = q.getInt(4), budget = q.getInt(5);
        if (deadline < 1 || deadline > 30_000 || budget < 0 || budget > 16)
            throw new Bridge.Limit();
        JSONArray rows = q.getJSONArray(6);
        if (rows.length() < 1 || rows.length() > 32) throw new Bridge.Limit();
        Ui ui = requireUi();
        Ui.Scene scene;
        List<P> plan = new ArrayList<>(rows.length());
        int mutations = 0;
        synchronized (ui.guard()) {
            scene = ui.snapshot();
            if (scene.generation != expected) throw new Stale();
            for (int i = 0; i < rows.length(); i++) {
                P p = P.parse(rows.getJSONArray(i), scene, context, capture);
                plan.add(p);
                if (p.mutates && ++mutations > budget) throw new Bridge.Limit();
            }
        }
        long end = SystemClock.elapsedRealtime() + deadline;
        int committed = 0, at = 0;
        boolean guarded = false;
        String artifact = null;
        try {
            for (int i = 0; i < plan.size(); i++) {
                P p = plan.get(i);
                at = i;
                if (SystemClock.elapsedRealtime() > end) throw new Fail(i, true);
                boolean ok;
                if (p.mutates && !guarded && p.unguarded()) {
                    // A long step must not hold the guard: it would block other workers' observe
                    // and semantic reads, and a capture would block taps, for its whole duration.
                    synchronized (ui.guard()) {
                        if (ui.snapshot().generation != expected) throw new Stale();
                    }
                    guarded = true;
                    ok = p.apply(context, ui, capture, end);
                } else if (p.mutates && !guarded) {
                    synchronized (ui.guard()) {
                        if (ui.snapshot().generation != expected) throw new Stale();
                        ok = p.apply(context, ui, capture, end);
                        guarded = true;
                    }
                } else {
                    // The generation moves on legitimate chains (type, then tap "Send"), so each
                    // later targeted change re-validates its own node instead (see current()).
                    if (p.mutates && p.node != null && !p.current(ui)) throw new Stale();
                    ok = p.apply(context, ui, capture, end);
                }
                // Scrolling that found nothing still moved the screen.
                if (!ok && p.scrolled) committed++;
                if (!ok) throw new Fail(i, false);
                if (p.mutates) committed++;
                if (p.artifact != null) artifact = p.artifact;
            }
        } catch (Stale e) {
            if (committed == 0) throw e;
            return new R(3, gen(), committed, at, artifact);
        } catch (Fail e) {
            return new R(committed > 0 ? 3 : e.timeout ? 2 : 9, gen(), committed, e.at, artifact);
        } catch (Exception e) {
            // A failure after a committed step must not read as "nothing ran": the
            // earlier steps already happened, so the receipt reports partial.
            if (committed == 0) throw e;
            return new R(3, gen(), committed, at, artifact);
        } finally {
            for (P p : plan) p.close();
        }
        return new R(0, gen(), committed, -1, artifact);
    }

    private static int failure(Exception e) {
        if (e instanceof Stale) return 1;
        if (e instanceof Timeout) return 2;
        if (e instanceof Bridge.Limit) return 6;
        return e instanceof Permission ? 10 : 5;
    }

    private static long gen() {
        Ui u = Ui.get();
        return u == null ? 0 : u.generation();
    }

    private static Ui requireUi() throws Unsupported {
        Ui u = Ui.get();
        if (u == null) throw new Unsupported();
        return u;
    }

    static final class R {
        final int code;
        final long g;
        final int mutations, at;
        final String artifact;

        R(int code, long g, int mutations, int at, String artifact) {
            this.code = code;
            this.g = g;
            this.mutations = mutations;
            this.at = at;
            this.artifact = artifact;
        }

        JSONArray wire(long seq) {
            JSONArray a = new JSONArray().put(seq).put(code).put(g).put(mutations);
            if (at >= 0) a.put(at);
            else if (artifact != null) a.put(org.json.JSONObject.NULL);
            if (artifact != null) a.put(artifact);
            return a;
        }
    }

    private static final int MAX_SCROLLS = 12;

    static final class P implements AutoCloseable {
        final String name;
        final AccessibilityNodeInfo node;
        final String text;
        final int arg;
        final Pred pred;
        final JSONArray points;
        final boolean mutates;
        final String seen;
        String artifact;
        boolean scrolled;
        long cap;

        P(
                String name,
                AccessibilityNodeInfo node,
                String text,
                int arg,
                Pred pred,
                JSONArray points,
                boolean mutates) {
            this.name = name;
            this.node = node;
            this.text = text;
            this.arg = arg;
            this.pred = pred;
            this.points = points;
            this.mutates = mutates;
            this.seen = node == null ? null : print(node);
        }

        /**
         * Whether the node is still what the plan resolved: in the active window (a dialog that
         * opened over it fails), visible, enabled, with the same view id and label. Bounds are not
         * compared: the keyboard opening after a type step resizes the layout under the target.
         */
        boolean current(Ui ui) {
            AccessibilityNodeInfo root = ui.getRootInActiveWindow();
            if (root == null) return false;
            boolean active = root.getWindowId() == node.getWindowId();
            root.recycle();
            return active
                    && node.refresh()
                    && node.isVisibleToUser()
                    && node.isEnabled()
                    && print(node).equals(seen);
        }

        /** Identity and label; a text field's own text is left out so typing keeps it. */
        private static String print(AccessibilityNodeInfo n) {
            return n.getViewIdResourceName()
                    + "|"
                    + (n.isEditable() ? "" : n.getText())
                    + "|"
                    + n.getContentDescription();
        }

        static P parse(JSONArray a, Ui.Scene s, Context context, Capture capture) throws Exception {
            if (a.length() < 1) throw new Bridge.Limit();
            String n = a.getString(0);
            switch (n) {
                case "tap":
                    exact(a, 2);
                    return ref(n, a, s, true);
                case "long":
                    exact(a, 2);
                    return ref(n, a, s, true);
                case "text":
                    exact(a, 3);
                    String t = bounded(a.getString(2), 8192);
                    return new P(n, copy(s, a.getInt(1)), t, 0, null, null, true);
                case "tap_point":
                    exact(a, 3);
                    return swipe(n, a.getInt(1), a.getInt(2), a.getInt(1), a.getInt(2), 1);
                case "swipe":
                    exact(a, 6);
                    return swipe(
                            n, a.getInt(1), a.getInt(2), a.getInt(3), a.getInt(4), a.getInt(5));
                case "scroll":
                    exact(a, 3);
                    String d = a.getString(2);
                    int dir =
                            "up".equals(d) || "left".equals(d)
                                    ? -1
                                    : "down".equals(d) || "right".equals(d) ? 1 : 0;
                    if (dir == 0) throw new Bridge.Limit();
                    return new P(n, copy(s, a.getInt(1)), d, dir, null, null, true);
                case "scroll_until":
                    exact(a, 4);
                    String ud = a.getString(2);
                    int udir = "up".equals(ud) || "left".equals(ud) ? -1 : 1;
                    if (!ud.matches("up|down|left|right")) throw new Bridge.Limit();
                    return new P(
                            n,
                            copy(s, a.getInt(1)),
                            ud,
                            udir,
                            Pred.parse(new JSONArray().put("text").put(a.getString(3)), s),
                            null,
                            true);
                case "settle":
                    exact(a, 4);
                    P settle =
                            new P(
                                    n,
                                    null,
                                    bounded(a.getString(3), 255),
                                    Math.min(Math.max(a.getInt(1), 0), 5_000),
                                    null,
                                    null,
                                    false);
                    settle.cap = Math.min(Math.max(a.getInt(2), 0), 10_000);
                    return settle;
                case "key":
                    exact(a, 2);
                    String k = a.getString(1);
                    if (!k.matches("back|home|recents|notifications|enter")
                            || (k.equals("enter") && Build.VERSION.SDK_INT < 30))
                        throw new Unsupported();
                    return new P(n, null, k, 0, null, null, true);
                case "gesture":
                    exact(a, 2);
                    JSONArray pts = a.getJSONArray(1);
                    if (pts.length() < 2 || pts.length() > 16) throw new Bridge.Limit();
                    for (int i = 0; i < pts.length(); i++) {
                        JSONArray p = pts.getJSONArray(i);
                        if (p.length() != 3) throw new Bridge.Limit();
                        xy(p.getInt(0), p.getInt(1), p.getInt(2));
                    }
                    return new P(n, null, null, 0, null, pts, true);
                case "wait":
                    exact(a, 3);
                    int ms = a.getInt(2);
                    if (ms < 0 || ms > 30_000) throw new Bridge.Limit();
                    return new P(n, null, null, ms, Pred.parse(a.getJSONArray(1), s), null, false);
                case "assert":
                    exact(a, 2);
                    return new P(n, null, null, 0, Pred.parse(a.getJSONArray(1), s), null, false);
                case "launch":
                    exact(a, 2);
                    String pkg = bounded(a.getString(1), 255);
                    Intent launch = context.getPackageManager().getLaunchIntentForPackage(pkg);
                    if (launch == null) throw new Unsupported();
                    return new P(n, null, pkg, 0, null, null, true);
                case "setting":
                    exact(a, 2);
                    String setting = bounded(a.getString(1), 128);
                    if (settingIntent(setting) == null) throw new Unsupported();
                    return new P(n, null, setting, 0, null, null, true);
                case "link":
                    exact(a, 2);
                    String link = bounded(a.getString(1), 2048);
                    if (!safeLink(link)) throw new Bridge.Limit();
                    return new P(n, null, link, 0, null, null, true);
                case "capture":
                    exact(a, 2);
                    if (!"screen".equals(a.getString(1)) || Build.VERSION.SDK_INT < 30)
                        throw new Unsupported();
                    return new P(n, null, "screen", 0, null, null, false);
                case "camera":
                    if (a.length() != 2 && a.length() != 4) throw new Bridge.Limit();
                    String facing = a.getString(1);
                    if (!"rear".equals(facing) && !"front".equals(facing) && !facing.isEmpty())
                        throw new Bridge.Limit();
                    if (a.length() == 4) {
                        int width = a.getInt(2), height = a.getInt(3);
                        if (width < 160 || width > 4096 || height < 160 || height > 4096)
                            throw new Bridge.Limit();
                        facing = facing + "|" + width + "x" + height;
                    }
                    return new P(n, null, facing, 0, null, null, true);
                case "microphone":
                    exact(a, 2);
                    int seconds = a.getInt(1);
                    if (seconds < 1 || seconds > 30) throw new Bridge.Limit();
                    return new P(n, null, "microphone", seconds, null, null, true);
                case "screen_record":
                    exact(a, 2);
                    int duration = a.getInt(1);
                    if (duration < 1 || duration > 30) throw new Bridge.Limit();
                    return new P(n, null, "screen_record", duration, null, null, true);
                case "notification_open":
                case "notification_dismiss":
                    exact(a, 2);
                    return new P(n, null, bounded(a.getString(1), 256), 0, null, null, true);
                case "notification_action":
                    if (a.length() != 2) exact(a, 3);
                    // Key and optional action label travel together, split by Notify.action.
                    String key = bounded(a.getString(1), 256);
                    if (a.length() == 3) key += "\u0001" + bounded(a.getString(2), 256);
                    return new P(n, null, key, 0, null, null, true);
                default:
                    throw new Unsupported();
            }
        }

        private static P ref(String n, JSONArray a, Ui.Scene s, boolean mut) throws Bridge.Limit {
            return new P(n, copy(s, a.optInt(1, -1)), null, 0, null, null, mut);
        }

        private static P swipe(String n, int x1, int y1, int x2, int y2, int duration)
                throws Bridge.Limit {
            JSONArray p = new JSONArray().put(xy(x1, y1, 1)).put(xy(x2, y2, duration));
            return new P(n, null, null, 0, null, p, true);
        }

        /** One bounded gesture point: screen coordinates and a time in milliseconds. */
        /**
         * Scrolls until the text is on screen: up to twelve pages, stopping at the end of the list.
         * True when found; scrolling that found nothing still counts as a change.
         */
        private boolean scrollUntil(Ui ui, long end) throws Exception {
            int action = Ui.scrollAction(text);
            for (int page = 0; page <= MAX_SCROLLS; page++) {
                if (pred.test(ui.snapshot())) return true;
                if (page == MAX_SCROLLS || SystemClock.elapsedRealtime() > end) break;
                long before = ui.generation();
                boolean moved =
                        Ui.supports(node, action)
                                ? node.performAction(action)
                                : Ui.pages(node, text)
                                        && node.performAction(
                                                arg < 0
                                                        ? AccessibilityNodeInfo
                                                                .ACTION_SCROLL_BACKWARD
                                                        : AccessibilityNodeInfo
                                                                .ACTION_SCROLL_FORWARD);
                if (!moved) break;
                scrolled = true;
                // Let the list settle: the scroll lands a moment later.
                long until = SystemClock.elapsedRealtime() + 400;
                while (ui.generation() == before && SystemClock.elapsedRealtime() < until)
                    Thread.sleep(25);
                Thread.sleep(120);
                node.refresh();
            }
            return false;
        }

        private static JSONArray xy(int x, int y, int ms) throws Bridge.Limit {
            if (x < 0 || y < 0 || x > 65_535 || y > 65_535 || ms < 0 || ms > 30_000)
                throw new Bridge.Limit();
            return new JSONArray().put(x).put(y).put(ms);
        }

        boolean apply(Context context, Ui ui, Capture capture, long end) throws Exception {
            switch (name) {
                case "tap":
                    if (node.isClickable() || Ui.supports(node, AccessibilityNodeInfo.ACTION_CLICK))
                        return node.performAction(AccessibilityNodeInfo.ACTION_CLICK);
                    // Plain text a screen reacts to (a list row without a click action): touch
                    // the middle of that same node, which was just checked to still be on screen.
                    Rect box = new Rect();
                    node.getBoundsInScreen(box);
                    if (box.isEmpty() || !node.isVisibleToUser()) return false;
                    return gesture(ui, new JSONArray().put(xy(box.centerX(), box.centerY(), 1)));
                case "long":
                    return node.performAction(AccessibilityNodeInfo.ACTION_LONG_CLICK);
                case "scroll_until":
                    return scrollUntil(ui, end);
                case "settle":
                    ui.settle(arg, cap, text, end);
                    return true;
                case "text":
                    Bundle b = new Bundle();
                    b.putCharSequence(
                            AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, text);
                    return node.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, b);
                case "scroll":
                    // Use the explicit direction when the view offers it, else forward/backward
                    // only when that pages along the requested axis.
                    int directional = Ui.scrollAction(text);
                    if (Ui.supports(node, directional)) return node.performAction(directional);
                    if (!Ui.pages(node, text)) return false;
                    return node.performAction(
                            arg < 0
                                    ? AccessibilityNodeInfo.ACTION_SCROLL_BACKWARD
                                    : AccessibilityNodeInfo.ACTION_SCROLL_FORWARD);
                case "key":
                    return key(ui, text);
                case "gesture":
                case "tap_point":
                case "swipe":
                    return gesture(ui, points);
                case "wait":
                    long until = Math.min(end, SystemClock.elapsedRealtime() + arg);
                    do {
                        if (pred.test(ui.snapshot())) return true;
                        Thread.sleep(50);
                    } while (SystemClock.elapsedRealtime() < until);
                    return false;
                case "assert":
                    return pred.test(ui.snapshot());
                case "launch":
                    Intent i = context.getPackageManager().getLaunchIntentForPackage(text);
                    if (i == null) return false;
                    i.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
                    context.startActivity(i);
                    return true;
                case "setting":
                    Intent setting = settingIntent(text);
                    if (setting == null) return false;
                    setting.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
                    context.startActivity(setting);
                    return true;
                case "link":
                    Intent link = new Intent(Intent.ACTION_VIEW, Uri.parse(text));
                    link.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
                    context.startActivity(link);
                    return true;
                case "capture":
                    artifact = capture.screen();
                    return artifact != null;
                case "camera":
                    artifact = capture.camera(text);
                    return artifact != null;
                case "microphone":
                    artifact = capture.microphone(arg);
                    return artifact != null;
                case "screen_record":
                    artifact = capture.screenRecord(arg);
                    return artifact != null;
                case "notification_open":
                    return Notify.open(text);
                case "notification_dismiss":
                    return Notify.dismiss(text);
                case "notification_action":
                    return Notify.action(text);
                default:
                    return false;
            }
        }

        private boolean unguarded() {
            return name.equals("scroll_until")
                    || name.equals("camera")
                    || name.equals("microphone")
                    || name.equals("screen_record");
        }

        @Override
        public void close() {
            if (node != null) node.recycle();
        }

        private static boolean key(Ui ui, String key) {
            switch (key) {
                case "back":
                    return ui.performGlobalAction(Ui.GLOBAL_ACTION_BACK);
                case "home":
                    return ui.performGlobalAction(Ui.GLOBAL_ACTION_HOME);
                case "recents":
                    return ui.performGlobalAction(Ui.GLOBAL_ACTION_RECENTS);
                case "notifications":
                    return ui.performGlobalAction(Ui.GLOBAL_ACTION_NOTIFICATIONS);
                case "enter":
                    AccessibilityNodeInfo root = ui.getRootInActiveWindow();
                    if (root == null) return false;
                    AccessibilityNodeInfo focus = root.findFocus(AccessibilityNodeInfo.FOCUS_INPUT);
                    boolean ok =
                            focus != null
                                    && focus.performAction(
                                            AccessibilityNodeInfo.AccessibilityAction
                                                    .ACTION_IME_ENTER
                                                    .getId());
                    if (focus != null) focus.recycle();
                    root.recycle();
                    return ok;
                default:
                    return false;
            }
        }

        private static boolean gesture(Ui ui, JSONArray pts) throws JSONException {
            Path path = new Path();
            JSONArray p = pts.getJSONArray(0);
            path.moveTo(p.getInt(0), p.getInt(1));
            long duration = 1;
            for (int i = 1; i < pts.length(); i++) {
                p = pts.getJSONArray(i);
                path.lineTo(p.getInt(0), p.getInt(1));
                duration = Math.max(duration, p.getLong(2));
            }
            GestureDescription g =
                    new GestureDescription.Builder()
                            .addStroke(
                                    new GestureDescription.StrokeDescription(
                                            path, 0, Math.min(duration, 30_000)))
                            .build();
            return ui.dispatchGesture(g, null, null);
        }

        private static AccessibilityNodeInfo copy(Ui.Scene s, int id) throws Bridge.Limit {
            Ui.N n = s.ref(id);
            if (n == null) throw new Bridge.Limit();
            return AccessibilityNodeInfo.obtain(n.node);
        }

        private static void exact(JSONArray a, int n) throws Bridge.Limit {
            if (a.length() != n) throw new Bridge.Limit();
        }
    }

    static final class Pred {
        final String kind, label;
        final Integer ref;
        final long generation;

        Pred(String kind, String label, Integer ref, long generation) {
            this.kind = kind;
            this.label = label;
            this.ref = ref;
            this.generation = generation;
        }

        static Pred parse(JSONArray a, Ui.Scene s) throws Exception {
            if (a.length() != 2) throw new Bridge.Limit();
            String k = a.getString(0);
            switch (k) {
                case "text":
                    return new Pred(k, needle(a.getString(1)), null, 0);
                case "generation_after":
                    return new Pred(k, null, null, a.getLong(1));
                case "exists":
                case "missing":
                    Object v = a.get(1);
                    if (v instanceof Number) {
                        int r = ((Number) v).intValue();
                        if (s.ref(r) == null) throw new Bridge.Limit();
                        return new Pred(k, null, r, s.generation);
                    }
                    JSONArray m = (JSONArray) v;
                    if (m.length() != 2 || !"label".equals(m.getString(0)))
                        throw new Bridge.Limit();
                    return new Pred(k, needle(m.getString(1)), null, 0);
                default:
                    throw new Unsupported();
            }
        }

        /** A label that normalizes to nothing ("[]", "…") names nothing, so it is refused. */
        private static String needle(String label) throws Bridge.Limit {
            if (Targets.normalize(label).isEmpty()) throw new Bridge.Limit();
            return bounded(label, 1024);
        }

        boolean test(Ui.Scene s) {
            boolean v;
            if ("text".equals(kind)) return s.label(label);
            if ("generation_after".equals(kind)) return s.generation > generation;
            if (ref != null) {
                // A ref names a node in the scene the plan was compiled against. Once the screen
                // changed, the same index can name a different node, so the check fails closed.
                if (s.generation != generation) return false;
                v = s.ref(ref) != null;
            } else {
                // Absence from a scene cut off at the walk's caps is unknown, so "missing" fails.
                if (s.truncated && !"exists".equals(kind)) return false;
                v = s.named(label);
            }
            return "exists".equals(kind) ? v : !v;
        }
    }

    private static String bounded(String s, int max) throws Bridge.Limit {
        if (s.getBytes(StandardCharsets.UTF_8).length > max) throw new Bridge.Limit();
        return s;
    }

    static final class Stale extends Exception {}

    static final class Timeout extends Exception {}

    static final class Unsupported extends Exception {}

    static final class Permission extends Exception {}

    static final class Fail extends Exception {
        final int at;
        final boolean timeout;

        Fail(int at, boolean timeout) {
            this.at = at;
            this.timeout = timeout;
        }
    }

    private static boolean safeLink(String value) {
        return (value.startsWith("https://")
                        || value.startsWith("http://")
                        || value.startsWith("geo:")
                        || value.startsWith("google.navigation:"))
                && !value.matches(".*[\\p{Cntrl}\\\"].*");
    }

    private static final String[][] SETTINGS = {
        {"accessibility", Settings.ACTION_ACCESSIBILITY_SETTINGS},
        {"wifi", Settings.ACTION_WIFI_SETTINGS},
        {"bluetooth", Settings.ACTION_BLUETOOTH_SETTINGS},
        {"display", Settings.ACTION_DISPLAY_SETTINGS},
        {"sound", Settings.ACTION_SOUND_SETTINGS},
        {"notifications", "android.settings.NOTIFICATION_SETTINGS"},
        {"apps", Settings.ACTION_APPLICATION_SETTINGS},
        {"battery", Settings.ACTION_BATTERY_SAVER_SETTINGS},
        {"date and time", Settings.ACTION_DATE_SETTINGS},
        {"developer options", Settings.ACTION_APPLICATION_DEVELOPMENT_SETTINGS}
    };

    private static Intent settingIntent(String value) {
        String v = value.toLowerCase(java.util.Locale.ROOT).trim();
        for (String[] setting : SETTINGS) if (setting[0].equals(v)) return new Intent(setting[1]);
        return null;
    }
}
