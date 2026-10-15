package dev.codex.aubridge;

import android.accessibilityservice.AccessibilityService;
import android.graphics.Rect;
import android.os.Build;
import android.os.SystemClock;
import android.view.accessibility.AccessibilityEvent;
import android.view.accessibility.AccessibilityNodeInfo;

import org.json.JSONArray;
import org.json.JSONObject;

import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import java.util.Locale;
import java.util.concurrent.atomic.AtomicLong;

public final class Ui extends AccessibilityService {
    private static volatile Ui instance;
    private final Object guard = new Object();
    // Events only bump these, never take the lock; a scene older than the generation is rebuilt on
    // demand.
    private final AtomicLong generation = new AtomicLong(1);
    private volatile long lastEvent;
    private volatile Scene scene;

    static Ui get() {
        return instance;
    }

    Object guard() {
        return guard;
    }

    @Override
    protected void onServiceConnected() {
        instance = this;
        invalidate();
    }

    @Override
    public void onAccessibilityEvent(AccessibilityEvent event) {
        if (shouldInvalidate(event.getEventType(), event.getContentChangeTypes())) {
            lastEvent = SystemClock.uptimeMillis();
            invalidate();
        }
    }

    /**
     * Waits for the screen to react to an action and then go quiet: first up to {@value
     * #REACTION_MS} ms for any event (an action's events arrive a moment after it; none means it
     * changed nothing), then until {@code quietMs} pass without one and, when {@code pkg} is not
     * empty, that app is in front. Gives up after {@code capMs} or at {@code deadline}. Best
     * effort: a screen that never settles is reported as it is.
     */
    void settle(long quietMs, long capMs, String pkg, long deadline) throws InterruptedException {
        long start = SystemClock.uptimeMillis();
        long limit = Math.min(start + capMs, start + deadline - SystemClock.elapsedRealtime());
        while (lastEvent < start
                && pkg.isEmpty()
                && SystemClock.uptimeMillis() - start < REACTION_MS
                && SystemClock.uptimeMillis() < limit) Thread.sleep(20);
        while (SystemClock.uptimeMillis() < limit) {
            long quiet = SystemClock.uptimeMillis() - Math.max(lastEvent, start);
            if (quiet >= quietMs && (pkg.isEmpty() || pkg.equals(activePackage()))) return;
            Thread.sleep(20);
        }
    }

    private static final long REACTION_MS = 450;

    @Override
    public void onInterrupt() {}

    @Override
    public void onDestroy() {
        if (instance == this) instance = null;
        clear();
        super.onDestroy();
    }

    /** Package of the window in front, so the host can tell a launched app has appeared. */
    String activePackage() {
        AccessibilityNodeInfo root = getRootInActiveWindow();
        if (root == null) return "";
        CharSequence name = root.getPackageName();
        root.recycle();
        return name == null ? "" : shorten(name.toString(), 255);
    }

    long generation() {
        return generation.get();
    }

    private void invalidate() {
        generation.incrementAndGet();
    }

    private void clear() {
        synchronized (guard) {
            clearLocked();
        }
    }

    private void clearLocked() {
        if (scene != null) {
            scene.recycle();
            scene = null;
        }
    }

    static boolean shouldInvalidate(int type, int changes) {
        if (type == AccessibilityEvent.TYPE_ANNOUNCEMENT
                || type == AccessibilityEvent.TYPE_NOTIFICATION_STATE_CHANGED) return false;
        if (type == AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED) {
            int relevant =
                    AccessibilityEvent.CONTENT_CHANGE_TYPE_TEXT
                            | AccessibilityEvent.CONTENT_CHANGE_TYPE_CONTENT_DESCRIPTION
                            | AccessibilityEvent.CONTENT_CHANGE_TYPE_STATE_DESCRIPTION
                            | AccessibilityEvent.CONTENT_CHANGE_TYPE_SUBTREE;
            return changes == AccessibilityEvent.CONTENT_CHANGE_TYPE_UNDEFINED
                    || (changes & relevant) != 0;
        }
        return type == AccessibilityEvent.TYPE_WINDOW_STATE_CHANGED
                || type == AccessibilityEvent.TYPE_VIEW_TEXT_CHANGED
                || type == AccessibilityEvent.TYPE_VIEW_CLICKED
                || type == AccessibilityEvent.TYPE_VIEW_SELECTED
                || type == AccessibilityEvent.TYPE_VIEW_FOCUSED
                || type == AccessibilityEvent.TYPE_VIEW_SCROLLED
                || type == AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUSED
                || type == AccessibilityEvent.TYPE_VIEW_CONTEXT_CLICKED;
    }

    Scene snapshot() {
        synchronized (guard) {
            if (scene == null || scene.generation != generation.get()) {
                clearLocked();
                scene = build();
            }
            return scene;
        }
    }

    JSONArray observe(long seq, String base, int detail) throws Bridge.Limit {
        Scene s = snapshot();
        if (base != null && base.equals(Long.toString(s.generation)))
            return new JSONArray().put(seq).put(0).put(s.generation);
        int limit = detail == 0 ? Math.min(s.frontier, 64) : Math.min(s.nodes.size(), 256),
                bytes = 0;
        JSONArray rows = new JSONArray();
        for (int i = 0; i < limit; i++) {
            N n = s.nodes.get(i);
            int cost = escaped(n.label) + 24;
            if (detail == 0 && rows.length() > 0 && bytes + cost > 3500) break;
            rows.put(n.wire());
            bytes += cost;
        }
        s.exposed = Math.min(s.frontier, rows.length());
        return new JSONArray().put(seq).put(0).put(s.generation).put(s.pkg).put(rows);
    }

    /** Handles shown in the last listing, by number: the node's key then (see {@link #key}). */
    private final java.util.Map<Integer, String> listed = new java.util.HashMap<>();

    /** Identity of a node across rebuilds of the scene: class, view id and bounds. */
    private static String key(N n) {
        return className(n)
                + "|"
                + n.node.getViewIdResourceName()
                + "|"
                + n.left
                + ","
                + n.top
                + ","
                + n.right
                + ","
                + n.bottom;
    }

    /** Remembers which node each listed row (by position in the full listing) was. */
    private void remember(List<SemanticCompiler.Row> rows, java.util.Map<Long, N> bySource) {
        java.util.Map<Integer, String> keys = new java.util.HashMap<>();
        for (int i = 0; i < rows.size(); i++) {
            N n = bySource.get(rows.get(i).source);
            if (n != null) keys.put(i, key(n));
        }
        synchronized (guard) {
            listed.clear();
            listed.putAll(keys);
        }
    }

    /**
     * Rows of the last listing that can be tapped, as {@code [handle, left, top, right, bottom]}.
     */
    JSONArray marks() {
        Scene s = snapshot();
        java.util.Map<Long, N> bySource = new java.util.HashMap<>();
        List<SemanticCompiler.Row> rows = compileSemantic(s, bySource);
        remember(rows, bySource);
        JSONArray out = new JSONArray();
        for (int i = 0; i < rows.size() && out.length() < 64; i++) {
            SemanticCompiler.Row row = rows.get(i);
            if (row.kind.isEmpty()
                    || row.kind.equals("heading")
                    || row.kind.equals("scroll area")
                    || row.right <= row.left) continue;
            out.put(
                    new JSONArray()
                            .put(i)
                            .put(row.left)
                            .put(row.top)
                            .put(row.right)
                            .put(row.bottom));
        }
        return out;
    }

    JSONArray semantic(String focus) throws Bridge.Limit {
        Scene s = snapshot();
        java.util.Map<Long, N> bySource = new java.util.HashMap<>();
        List<SemanticCompiler.Row> rows = compileSemantic(s, bySource);
        remember(rows, bySource);
        String needle = Targets.normalize(focus);
        JSONArray out = new JSONArray();
        int bytes = 0;
        for (int index = 0; index < rows.size(); index++) {
            SemanticCompiler.Row row = rows.get(index);
            String label = shorten(row.label, 512),
                    value = shorten(row.value, 512),
                    kind = shorten(row.kind, 64),
                    state = shorten(row.state, 64);
            if (!needle.isEmpty()
                    && !Targets.normalize(label + " " + value + " " + state + " " + kind)
                            .contains(needle)) continue;
            int cost = escaped(label) + escaped(value) + escaped(state) + escaped(kind) + 48;
            if (out.length() > 0 && bytes + cost > 18_000) break;
            out.put(
                    new JSONArray()
                            .put(label)
                            .put(value)
                            .put(kind)
                            .put(state)
                            .put(row.enabled)
                            .put(row.selected)
                            .put(index));
            bytes += cost;
        }
        return out;
    }

    /**
     * Owners of the controls that {@code label} names, in listing order, as {@code {r: refs, t:
     * tier, l: matched labels}}. A tier of 2 or more means no control had exactly that label.
     */
    JSONObject resolve(String label, String want, int handle) throws Bridge.Limit {
        Scene s = snapshot();
        s.exposed = Math.min(s.frontier, s.nodes.size());
        if (handle >= 0) return resolveHandle(s, handle);
        List<String[]> names = new ArrayList<>(s.nodes.size());
        for (N n : s.nodes) names.add(new String[] {n.semanticLabel, n.hint});
        Targets.Hits found = Targets.search(names, label);
        List<N> owners = new ArrayList<>();
        List<String> labels = new ArrayList<>();
        for (int index : found.hits) {
            N node = s.nodes.get(index);
            // A scroll step names the area itself, never the control drawn over its middle.
            N owner =
                    "scroll".equals(want)
                            ? (node.node.isScrollable() ? node : null)
                            : owner(s, node);
            if (owner != null && !owners.contains(owner)) {
                owners.add(owner);
                labels.add(
                        found.tier % 2 == 0
                                ? s.nodes.get(index).semanticLabel
                                : s.nodes.get(index).hint);
            }
        }
        // Plain text that shares its words with a control (a title above the field it names) is not
        // a candidate when the control is there: a step needing a field takes only fields, any
        // other takes only controls.
        List<Integer> keep = new ArrayList<>();
        for (int i = 0; i < owners.size(); i++) {
            N n = owners.get(i);
            if ("scroll".equals(want)
                    || ("edit".equals(want) ? n.node.isEditable() : n.interactive())) keep.add(i);
        }
        if (!keep.isEmpty() && keep.size() < owners.size()) {
            List<N> ownersKept = new ArrayList<>();
            List<String> labelsKept = new ArrayList<>();
            for (int i : keep) {
                ownersKept.add(owners.get(i));
                labelsKept.add(labels.get(i));
            }
            owners = ownersKept;
            labels = labelsKept;
        }
        // A row and the switch inside it both carry the row's words; they are one target, the
        // inner one.
        List<N> listed = new ArrayList<>();
        List<String> shownLabels = new ArrayList<>();
        for (int i = 0; i < owners.size(); i++) {
            boolean wraps = false;
            for (int j = 0; j < owners.size() && !wraps; j++) {
                wraps = j != i && owners.get(i).wraps(owners.get(j));
            }
            if (!wraps) {
                listed.add(owners.get(i));
                shownLabels.add(labels.get(i));
            }
        }
        labels = shownLabels;
        // Top to bottom, then left to right: the order the screen listing numbers duplicates in.
        List<Integer> order = new ArrayList<>();
        for (int i = 0; i < listed.size(); i++) order.add(i);
        order.sort(
                Comparator.comparingInt((Integer i) -> listed.get(i).top)
                        .thenComparingInt(i -> listed.get(i).left));
        JSONArray refs = new JSONArray(), names2 = new JSONArray();
        for (int i : order) {
            if (refs.length() >= 64) break;
            refs.put(listed.get(i).id);
            names2.put(shorten(labels.get(i), 96));
        }
        try {
            return new JSONObject().put("r", refs).put("t", found.tier).put("l", names2);
        } catch (org.json.JSONException e) {
            throw new Bridge.Limit();
        }
    }

    /** The node a handle from the last listing named, found again by its key; none if it moved. */
    private JSONObject resolveHandle(Scene s, int handle) throws Bridge.Limit {
        String key;
        synchronized (guard) {
            key = listed.get(handle);
        }
        JSONArray refs = new JSONArray(), shown = new JSONArray();
        if (key != null) {
            for (N n : s.nodes) {
                if (n.id < s.frontier && key.equals(key(n))) {
                    refs.put(n.id);
                    shown.put("e" + handle);
                    break;
                }
            }
        }
        try {
            return new JSONObject().put("r", refs).put("t", 0).put("l", shown);
        } catch (org.json.JSONException e) {
            throw new Bridge.Limit();
        }
    }

    /**
     * The area a bare "scroll DIRECTION" should move: the largest visible scrollable node that
     * advertises that direction, or the largest scrollable node when none advertise directions.
     */
    JSONArray scroller(String direction) {
        Scene s = snapshot();
        s.exposed = Math.min(s.frontier, s.nodes.size());
        int action = scrollAction(direction);
        N best = null;
        boolean bestDirectional = false;
        long bestArea = -1;
        for (int i = 0; i < Math.min(s.frontier, s.nodes.size()); i++) {
            N n = s.nodes.get(i);
            if (!n.node.isScrollable()) continue;
            boolean directional = supports(n.node, action);
            if (!directional && !pages(n.node, direction)) continue;
            long area = (long) Math.max(0, n.right - n.left) * Math.max(0, n.bottom - n.top);
            if (best == null
                    || (directional && !bestDirectional)
                    || (directional == bestDirectional && area > bestArea)) {
                best = n;
                bestDirectional = directional;
                bestArea = area;
            }
        }
        JSONArray out = new JSONArray();
        if (best != null) out.put(best.id);
        return out;
    }

    static int scrollAction(String direction) {
        switch (direction) {
            case "up":
                return AccessibilityNodeInfo.AccessibilityAction.ACTION_SCROLL_UP.getId();
            case "left":
                return AccessibilityNodeInfo.AccessibilityAction.ACTION_SCROLL_LEFT.getId();
            case "right":
                return AccessibilityNodeInfo.AccessibilityAction.ACTION_SCROLL_RIGHT.getId();
            default:
                return AccessibilityNodeInfo.AccessibilityAction.ACTION_SCROLL_DOWN.getId();
        }
    }

    /**
     * Whether forward/backward scrolls the requested axis: only on views without explicit
     * directions, and horizontally only on views known to be horizontal.
     */
    static boolean pages(AccessibilityNodeInfo node, String direction) {
        return !advertisesDirections(node)
                && ("left".equals(direction) || "right".equals(direction)) == horizontal(node);
    }

    private static boolean horizontal(AccessibilityNodeInfo node) {
        AccessibilityNodeInfo.CollectionInfo info = node.getCollectionInfo();
        String c = node.getClassName() == null ? "" : node.getClassName().toString();
        return (info != null && info.getRowCount() == 1 && info.getColumnCount() > 1)
                || c.endsWith("HorizontalScrollView")
                || c.endsWith(".ViewPager");
    }

    /** Whether the view advertises any explicit scroll direction (API 23+ views usually do). */
    static boolean advertisesDirections(AccessibilityNodeInfo node) {
        for (String direction : new String[] {"up", "down", "left", "right"}) {
            if (supports(node, scrollAction(direction))) return true;
        }
        return false;
    }

    static boolean supports(AccessibilityNodeInfo node, int action) {
        for (AccessibilityNodeInfo.AccessibilityAction candidate : node.getActionList()) {
            if (candidate.getId() == action) return true;
        }
        return false;
    }

    private static N owner(Scene s, N match) {
        if (match.interactive() && !match.node.isScrollable()) return match;
        N fallback = null;
        for (N candidate : s.nodes) {
            if (candidate.sourceId == match.parentSourceId && candidate.interactive()) {
                if (!candidate.node.isScrollable()) return candidate;
                fallback = candidate;
            }
        }
        // A sibling owns the match only when it covers it; a neighbouring control (a row's
        // "Delete" button beside its title) is a different target.
        for (N candidate : s.nodes) {
            if (candidate.parentSourceId == match.parentSourceId
                    && candidate.interactive()
                    && candidate.contains(match)) {
                if (!candidate.node.isScrollable()) return candidate;
                fallback = candidate;
            }
        }
        N spatial = spatialOwner(s, match);
        return spatial != null ? spatial : (fallback == null ? match : fallback);
    }

    /**
     * The smallest clickable control whose bounds contain the match's center, or null. A control is
     * never chosen merely for being nearby: falling back to the nearest control on screen could tap
     * an unrelated button, such as a destructive one.
     */
    private static N spatialOwner(Scene s, N match) {
        N best = null;
        long bestArea = Long.MAX_VALUE;
        for (N candidate : s.nodes) {
            if (!candidate.node.isClickable() || candidate.node.isScrollable()) continue;
            long area =
                    Math.max(
                            1L,
                            (long) (candidate.right - candidate.left)
                                    * (candidate.bottom - candidate.top));
            if (candidate.contains(match) && (best == null || area < bestArea)) {
                best = candidate;
                bestArea = area;
            }
        }
        return best;
    }

    private static List<SemanticCompiler.Row> compileSemantic(
            Scene s, java.util.Map<Long, N> bySource) {
        List<SemanticCompiler.Item> input = new ArrayList<>();
        for (N n : s.nodes) {
            bySource.put(n.sourceId >= 0 ? n.sourceId : n.id, n);
            String c = className(n), lower = c.toLowerCase(Locale.ROOT);
            input.add(
                    new SemanticCompiler.Item(
                            n.sourceId >= 0 ? n.sourceId : n.id,
                            n.parentSourceId,
                            n.semanticLabel,
                            n.hint,
                            c,
                            n.node.isClickable(),
                            n.node.isEnabled(),
                            n.node.isCheckable(),
                            n.node.isChecked(),
                            n.node.isEditable(),
                            n.password,
                            n.hasText,
                            n.node.isScrollable(),
                            n.node.isSelected(),
                            isHeading(n),
                            lower.contains("button"),
                            lower.contains("link"),
                            lower.contains("radio"),
                            lower.contains("checkbox"),
                            lower.contains("seekbar") || lower.contains("ratingbar"),
                            true,
                            false,
                            n.left,
                            n.top,
                            n.right,
                            n.bottom));
        }
        return SemanticCompiler.compile(input);
    }

    private static boolean isHeading(N n) {
        return Build.VERSION.SDK_INT >= 28 && n.node.isHeading();
    }

    private static String className(N n) {
        return n.node.getClassName() == null ? "" : n.node.getClassName().toString();
    }

    private Scene build() {
        long generation = this.generation.get();
        AccessibilityNodeInfo root = getRootInActiveWindow();
        if (root == null) return new Scene(generation, "", new ArrayList<>(), 0, false);
        String pkg = root.getPackageName() == null ? "" : root.getPackageName().toString();
        ArrayDeque<Visit> q = new ArrayDeque<>();
        long nextSource = 1;
        q.add(new Visit(root, nextSource++, -1));
        List<N> frontier = new ArrayList<>(), rest = new ArrayList<>();
        int seen = 0;
        while (!q.isEmpty() && seen++ < 512) {
            Visit visit = q.removeFirst();
            AccessibilityNodeInfo n = visit.node;
            for (int i = 0; i < n.getChildCount() && q.size() < 512; i++) {
                AccessibilityNodeInfo c = n.getChild(i);
                if (c != null) q.addLast(new Visit(c, nextSource++, visit.sourceId));
            }
            if (n.isVisibleToUser()) {
                N row = N.from(n, visit.sourceId, visit.parentSourceId);
                if (row.decision()) frontier.add(row);
                else rest.add(row);
            } else n.recycle();
        }
        boolean truncated = !q.isEmpty() || frontier.size() + rest.size() > 256;
        while (!q.isEmpty()) q.removeFirst().recycle();
        List<N> all = new ArrayList<>(Math.min(256, frontier.size() + rest.size()));
        for (N n : frontier) {
            if (all.size() == 256) {
                n.recycle();
                continue;
            }
            all.add(n);
        }
        int front = all.size();
        for (N n : rest) {
            if (all.size() == 256) {
                n.recycle();
                continue;
            }
            all.add(n);
        }
        for (int i = 0; i < all.size(); i++) all.get(i).id = i;
        return new Scene(generation, shorten(pkg, 255), all, front, truncated);
    }

    static final class Visit {
        final AccessibilityNodeInfo node;
        final long sourceId, parentSourceId;

        Visit(AccessibilityNodeInfo node, long sourceId, long parentSourceId) {
            this.node = node;
            this.sourceId = sourceId;
            this.parentSourceId = parentSourceId;
        }

        void recycle() {
            node.recycle();
        }
    }

    static final class Scene {
        final long generation;
        final String pkg;
        final List<N> nodes;
        final int frontier;
        final boolean truncated; // walk caps were hit, so absence from nodes proves nothing
        volatile int exposed;

        Scene(long generation, String pkg, List<N> nodes, int frontier, boolean truncated) {
            this.generation = generation;
            this.pkg = pkg;
            this.nodes = nodes;
            this.frontier = frontier;
            this.truncated = truncated;
        }

        N ref(int id) {
            return id >= 0 && id < frontier && id < exposed ? nodes.get(id) : null;
        }

        boolean label(String text) {
            String find = Targets.normalize(text);
            if (find.isEmpty()) return false; // "[]" or "…" names nothing, not everything
            for (N n : nodes) if (Targets.normalize(n.label).contains(find)) return true;
            return false;
        }

        /**
         * Like {@link #label}, but also matches text-field hints, so "verify" and "wait for" find a
         * filled field by the same name the resolver accepts for it.
         */
        boolean named(String text) {
            String find = Targets.normalize(text);
            if (find.isEmpty()) return false;
            for (N n : nodes) {
                if (Targets.normalize(n.label).contains(find)
                        || Targets.normalize(n.hint).contains(find)) return true;
            }
            return false;
        }

        void recycle() {
            for (N n : nodes) n.recycle();
        }
    }

    static final class N {
        int id;
        final String label;
        final int role, flags;
        final AccessibilityNodeInfo node;
        final int left, top, right, bottom;
        final long sourceId, parentSourceId;
        final String semanticLabel, hint;
        final boolean password, hasText;

        N(
                String label,
                int role,
                int flags,
                AccessibilityNodeInfo node,
                long sourceId,
                long parentSourceId,
                String semanticLabel,
                String hint,
                boolean password,
                boolean hasText,
                int left,
                int top,
                int right,
                int bottom) {
            this.label = label;
            this.role = role;
            this.flags = flags;
            this.node = node;
            this.sourceId = sourceId;
            this.parentSourceId = parentSourceId;
            this.semanticLabel = semanticLabel;
            this.hint = hint;
            this.password = password;
            this.hasText = hasText;
            this.left = left;
            this.top = top;
            this.right = right;
            this.bottom = bottom;
        }

        static N from(AccessibilityNodeInfo n, long source, long parent) {
            CharSequence t = n.getText(), d = n.getContentDescription(), h = n.getHintText();
            String text = nonEmpty(t),
                    desc = nonEmpty(d),
                    hint = nonEmpty(h),
                    r = n.getViewIdResourceName();
            boolean password = n.isPassword() || Secret.password(n.getInputType()),
                    hasText = !text.isEmpty();
            String semantic = shorten(password ? "" : (!text.isEmpty() ? text : desc), 1024);
            String label = shorten(!semantic.isEmpty() ? semantic : r != null ? r : "", 1024);
            Rect bounds = new Rect();
            n.getBoundsInScreen(bounds);
            int flags =
                    (n.isClickable() ? 1 : 0)
                            | (n.isEnabled() ? 2 : 0)
                            | (n.isChecked() ? 4 : 0)
                            | (n.isScrollable() ? 8 : 0);
            String c = n.getClassName() == null ? "" : n.getClassName().toString();
            int role =
                    n.isEditable()
                            ? 'i'
                            : c.contains("Button")
                                    ? 'b'
                                    : n.isCheckable() || c.contains("Switch")
                                            ? 'c'
                                            : n.isScrollable()
                                                    ? 's'
                                                    : !label.isEmpty()
                                                            ? 't'
                                                            : n.isClickable() ? 'm' : 'u';
            return new N(
                    label,
                    role,
                    flags,
                    n,
                    source,
                    parent,
                    semantic,
                    hint,
                    password,
                    hasText,
                    bounds.left,
                    bounds.top,
                    bounds.right,
                    bounds.bottom);
        }

        private static String nonEmpty(CharSequence value) {
            return value == null ? "" : value.toString().trim();
        }

        boolean decision() {
            return !label.isEmpty() || (flags & 9) != 0 || node.isEditable() || node.isCheckable();
        }

        /** Whether {@code inner} lies fully inside this node and is a different, smaller node. */
        boolean wraps(N inner) {
            return inner != this
                    && left <= inner.left
                    && top <= inner.top
                    && right >= inner.right
                    && bottom >= inner.bottom
                    && (right - left) * (bottom - top)
                            > (inner.right - inner.left) * (inner.bottom - inner.top);
        }

        boolean contains(N other) {
            int cx = (other.left + other.right) / 2, cy = (other.top + other.bottom) / 2;
            return cx >= left && cx <= right && cy >= top && cy <= bottom;
        }

        boolean interactive() {
            return node.isClickable()
                    || node.isCheckable()
                    || node.isEditable()
                    || node.isScrollable();
        }

        JSONArray wire() {
            return new JSONArray().put(id).put(label).put(role).put(flags);
        }

        void recycle() {
            node.recycle();
        }
    }

    private static int utf8(int cp) {
        return cp < 0x80
                ? 1
                : cp < 0x800
                        ? 2
                        : cp < 0xD800 || cp > 0xDFFF && cp < 0x10000 ? 3 : cp < 0x10000 ? 1 : 4;
    }

    private static String shorten(String s, int max) {
        int bytes = 0, end = 0;
        while (end < s.length()) {
            int cp = s.codePointAt(end), n = utf8(cp);
            if (bytes + n > max) break;
            bytes += n;
            end += Character.charCount(cp);
        }
        return end == s.length() ? s : s.substring(0, end);
    }

    private static int escaped(String s) {
        int bytes = 0;
        for (int i = 0; i < s.length(); ) {
            int cp = s.codePointAt(i);
            bytes += cp < 32 ? 6 : cp == '"' || cp == '\\' ? 2 : utf8(cp);
            i += Character.charCount(cp);
        }
        return bytes;
    }
}
