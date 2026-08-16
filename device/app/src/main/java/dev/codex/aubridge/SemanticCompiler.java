package dev.codex.aubridge;

import java.util.ArrayList;
import java.util.Collections;
import java.util.Comparator;
import java.util.HashSet;
import java.util.IdentityHashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;

/**
 * Converts accessibility evidence into small conceptual rows. It deliberately
 * knows nothing about Android package names, resource identifiers, or app
 * labels; those are implementation evidence, never semantic rules.
 */
final class SemanticCompiler {
    private SemanticCompiler() {}

    static final class Item {
        final long id, parent;
        final String label, hint, role;
        final boolean clickable, enabled, checkable, checked, editable, password, hasText, scrollable, selected, heading, button, link, radio, checkbox, slider, visible, modal;
        final int left, top, right, bottom;

        Item(long id, long parent, String label, String hint, String role, boolean clickable, boolean enabled, boolean checkable, boolean checked, boolean editable, boolean password, boolean hasText, boolean scrollable, boolean selected, boolean heading, boolean button, boolean link, boolean radio, boolean checkbox, boolean slider, boolean visible, boolean modal) {
            this(id, parent, label, hint, role, clickable, enabled, checkable, checked, editable, password, hasText, scrollable, selected, heading, button, link, radio, checkbox, slider, visible, modal, 0, 0, 0, 0);
        }

        Item(long id, long parent, String label, String hint, String role, boolean clickable, boolean enabled, boolean checkable, boolean checked, boolean editable, boolean password, boolean hasText, boolean scrollable, boolean selected, boolean heading, boolean button, boolean link, boolean radio, boolean checkbox, boolean slider, boolean visible, boolean modal, int left, int top, int right, int bottom) {
            this.id = id;
            this.parent = parent;
            this.label = clean(label);
            this.hint = clean(hint);
            this.role = clean(role);
            this.clickable = clickable;
            this.enabled = enabled;
            this.checkable = checkable;
            this.checked = checked;
            this.editable = editable;
            this.password = password;
            this.hasText = hasText;
            this.scrollable = scrollable;
            this.selected = selected;
            this.heading = heading;
            this.button = button;
            this.link = link;
            this.radio = radio;
            this.checkbox = checkbox;
            this.slider = slider;
            this.visible = visible;
            this.modal = modal;
            this.left = left;
            this.top = top;
            this.right = right;
            this.bottom = bottom;
        }

        boolean interactive() {
            return clickable || checkable || editable || scrollable || slider;
        }

        boolean hasBounds() {
            return right > left && bottom > top;
        }

        int centerX() {
            return left + Math.max(0, right - left) / 2;
        }

        int centerY() {
            return top + Math.max(0, bottom - top) / 2;
        }

        long area() {
            return hasBounds() ? (long) (right - left) * (bottom - top) : 0L;
        }
    }

    static final class Row {
        final String label, value, kind, state;
        final boolean enabled, selected;

        Row(String label, String value, String kind, String state, boolean enabled, boolean selected) {
            this.label = label;
            this.value = value;
            this.kind = kind;
            this.state = state;
            this.enabled = enabled;
            this.selected = selected;
        }
    }

    static List<Row> compile(List<Item> source) {
        List<Item> visible = new ArrayList<>();
        boolean modal = false;
        for (Item item : source) {
            if (item.visible && item.modal) modal = true;
        }
        for (Item item : source) {
            if (item.visible && (!modal || item.modal)) visible.add(item);
        }

        Map<Long, List<Item>> groups = new LinkedHashMap<>();
        for (Item item : visible) {
            if (!item.interactive() && item.label.isEmpty()) continue;
            long key = item.parent >= 0 ? item.parent : item.id;
            groups.computeIfAbsent(key, ignored -> new ArrayList<>()).add(item);
        }

        List<Row> rows = new ArrayList<>();
        Item heading = firstHeading(visible);
        if (heading != null) rows.add(new Row(heading.label, "", "heading", "", heading.enabled, heading.selected));

        Set<Long> consumed = new HashSet<>();
        List<Owned> owned = spatialRows(visible, heading, consumed);
        Collections.sort(owned, Comparator.comparingInt((Owned value) -> top(value.owner)).thenComparingInt(value -> value.owner.left));
        for (Owned row : owned) rows.add(row(row));

        for (List<Item> group : groups.values()) {
            List<Item> labels = new ArrayList<>();
            List<Item> values = new ArrayList<>();
            List<Item> controls = new ArrayList<>();
            for (Item item : group) {
                if (consumed.contains(item.id)) continue;
                if (item.interactive()) controls.add(item);
                if (!item.label.isEmpty() && item != heading) {
                    if (item.editable) values.add(item);
                    else labels.add(item);
                }
            }
            Item control = bestControl(controls);
            if (labels.isEmpty() && control != null && !control.label.isEmpty()) labels.add(control);
            if (labels.isEmpty()) continue;

            if (controls.size() > 1) {
                boolean independentlyLabeled = true;
                for (Item item : controls) if (item.label.isEmpty()) independentlyLabeled = false;
                if (independentlyLabeled) {
                    for (Item item : controls) {
                        rows.add(new Row(item.label, "", kind(item, item), state(item, item.selected), item.enabled, item.selected));
                    }
                    continue;
                }
            }

            String label = labels.get(0).label;
            String value = "";
            if (control != null && control.editable) {
                if (!control.hint.isEmpty()) label = control.hint;
                if (control.password) {
                    value = control.hasText ? "filled password" : "empty password";
                } else if (!control.label.isEmpty()) {
                    value = control.label;
                } else if (!values.isEmpty()) {
                    value = values.get(0).label;
                }
                if (label.isEmpty()) label = "Unlabeled text field";
            } else if (labels.size() > 1) {
                value = join(labels, 1);
            }

            boolean enabled = control == null ? labels.get(0).enabled : control.enabled;
            boolean selected = control != null && control.selected;
            String kind = kind(control, labels.get(0));
            String state = state(control, selected);
            rows.add(new Row(label, value, kind, state, enabled, selected));
        }
        return rows;
    }

    /**
     * Row containers and their text children do not always share one immediate
     * parent. Use the geometry as a second source of structure, assigning each
     * readable node to the smallest bounded clickable owner that contains it.
     * This is deliberately app-independent: it works for list rows, cards,
     * dialogs, and custom views without knowing their resource names.
     */
    private static List<Owned> spatialRows(List<Item> items, Item heading, Set<Long> consumed) {
        List<Item> owners = new ArrayList<>();
        int minLeft = Integer.MAX_VALUE, minTop = Integer.MAX_VALUE;
        int maxRight = Integer.MIN_VALUE, maxBottom = Integer.MIN_VALUE;
        for (Item item : items) {
            if (!item.visible || !item.hasBounds()) continue;
            minLeft = Math.min(minLeft, item.left);
            minTop = Math.min(minTop, item.top);
            maxRight = Math.max(maxRight, item.right);
            maxBottom = Math.max(maxBottom, item.bottom);
        }
        long canvasArea = maxRight > minLeft && maxBottom > minTop
            ? (long) (maxRight - minLeft) * (maxBottom - minTop) : 0L;
        for (Item item : items) {
            if (!item.visible || !item.hasBounds() || !item.clickable || item.scrollable) continue;
            if (canvasArea > 0 && item.area() * 100L >= canvasArea * 85L && item.label.isEmpty()) continue;
            owners.add(item);
        }
        if (owners.isEmpty()) return new ArrayList<>();

        Map<Item, List<Item>> members = new IdentityHashMap<>();
        for (Item owner : owners) members.put(owner, new ArrayList<>());
        for (Item item : items) {
            if (item == heading || !item.visible) continue;
            Item owner = bestOwner(item, owners);
            if (owner != null && usefulMember(item, owner)) members.get(owner).add(item);
        }
        List<Owned> result = new ArrayList<>();
        for (Item owner : owners) {
            List<Item> grouped = members.get(owner);
            if (!hasReadableContent(grouped)) continue;
            consumed.add(owner.id);
            for (Item member : grouped) consumed.add(member.id);
            result.add(new Owned(owner, grouped));
        }
        return result;
    }

    private static boolean usefulMember(Item item, Item owner) {
        return item == owner || !item.label.isEmpty() || (item.interactive() && !item.scrollable);
    }

    private static boolean hasReadableContent(List<Item> members) {
        for (Item item : members) {
            if (!item.label.isEmpty()) return true;
        }
        return false;
    }

    private static Item bestOwner(Item item, List<Item> owners) {
        Item best = null;
        long bestArea = Long.MAX_VALUE;
        for (Item owner : owners) {
            if (item == owner) return owner;
            if (!item.hasBounds() || !owner.hasBounds()) continue;
            int cx = item.centerX(), cy = item.centerY();
            if (cx < owner.left || cx > owner.right || cy < owner.top || cy > owner.bottom) continue;
            long area = owner.area();
            if (best == null || area < bestArea) {
                best = owner;
                bestArea = area;
            }
        }
        if (best != null) return best;

        // Bounds can be empty for off-screen/recycled children. Parentage is a
        // safe proximity fallback, but never attach an unrelated screen item.
        for (Item owner : owners) {
            if (item.parent == owner.id) return owner;
        }
        return null;
    }

    private static int top(Item item) {
        return item.hasBounds() ? item.top : Integer.MAX_VALUE;
    }

    private static final class Owned {
        final Item owner;
        final List<Item> members;

        Owned(Item owner, List<Item> members) {
            this.owner = owner;
            this.members = members;
        }
    }

    private static final class Text {
        final String value;
        final Item evidence;

        Text(String value, Item evidence) {
            this.value = value;
            this.evidence = evidence;
        }
    }

    private static Row row(Owned owned) {
        List<Item> members = new ArrayList<>(owned.members);
        Collections.sort(members, Comparator.comparingInt(SemanticCompiler::top).thenComparingInt(value -> value.left));
        List<Item> controls = new ArrayList<>();
        for (Item item : members) if (item.interactive() && !item.scrollable) controls.add(item);
        Item control = bestControl(controls);
        List<Text> texts = meaningfulTexts(members);
        if (texts.isEmpty() && control != null && !control.label.isEmpty()) texts.add(new Text(control.label, control));
        if (texts.isEmpty()) return new Row("", "", "", "", true, false);

        String label = texts.get(0).value;
        String value = joinTexts(texts, 1);
        if (control != null && control.editable) {
            if (!control.hint.isEmpty()) label = control.hint;
            if (control.password) value = control.hasText ? "filled password" : "empty password";
            else if (!control.label.isEmpty()) value = control.label;
        }
        if (label.isEmpty()) label = "Unlabeled text field";
        Item labelEvidence = texts.get(0).evidence;
        boolean enabled = control == null ? labelEvidence.enabled : control.enabled;
        boolean selected = control != null && control.selected;
        return new Row(label, value, kind(control, labelEvidence), state(control, selected), enabled, selected);
    }

    private static List<Text> meaningfulTexts(List<Item> members) {
        List<Item> labels = new ArrayList<>();
        for (Item item : members) if (!item.label.isEmpty() && !decorativeLabel(item, members)) labels.add(item);
        if (labels.isEmpty()) for (Item item : members) if (!item.label.isEmpty()) labels.add(item);
        List<Text> primary = new ArrayList<>(), residual = new ArrayList<>();
        for (Item item : labels) {
            if (redundantComposite(item, labels)) {
                for (String part : item.label.split(",")) {
                    String clean = clean(part);
                    if (!clean.isEmpty() && !matchesOther(clean, item, labels) && !containsText(residual, clean)) residual.add(new Text(clean, item));
                }
            } else if (!containsText(primary, item.label)) {
                primary.add(new Text(item.label, item));
            }
        }
        primary.addAll(residual);
        return primary;
    }

    private static boolean decorativeLabel(Item item, List<Item> members) {
        if (item.interactive()) return false;
        String role = item.role.toLowerCase(Locale.ROOT);
        if (!role.contains("image") && !role.contains("icon")) return false;
        int labels = 0;
        for (Item member : members) if (!member.label.isEmpty()) labels++;
        return labels > 1;
    }

    private static boolean redundantComposite(Item item, List<Item> labels) {
        if (item.label.indexOf(',') < 0) return false;
        String lower = item.label.toLowerCase(Locale.ROOT);
        for (Item other : labels) {
            if (other == item || other.label.length() < 3) continue;
            String candidate = other.label.toLowerCase(Locale.ROOT);
            if (!candidate.equals(lower) && lower.contains(candidate)) return true;
        }
        return false;
    }

    private static boolean matchesOther(String value, Item item, List<Item> labels) {
        String normalized = value.toLowerCase(Locale.ROOT);
        for (Item other : labels) {
            if (other != item && normalized.equals(other.label.toLowerCase(Locale.ROOT))) return true;
        }
        return false;
    }

    private static boolean containsText(List<Text> texts, String value) {
        String normalized = value.toLowerCase(Locale.ROOT);
        for (Text text : texts) if (text.value.toLowerCase(Locale.ROOT).equals(normalized)) return true;
        return false;
    }

    private static String joinTexts(List<Text> texts, int start) {
        StringBuilder out = new StringBuilder();
        for (int i = start; i < texts.size(); i++) {
            if (out.length() > 0) out.append(", ");
            out.append(texts.get(i).value);
        }
        return out.toString();
    }

    private static Item firstHeading(List<Item> items) {
        for (Item item : items) if (item.heading && !item.label.isEmpty()) return item;
        for (Item item : items) {
            if (!item.label.isEmpty() && !item.interactive()) return item;
        }
        return null;
    }

    private static Item bestControl(List<Item> controls) {
        Item fallback = null;
        for (Item item : controls) {
            if (item.checkable || item.editable || item.slider) return item;
            if (fallback == null) fallback = item;
        }
        return fallback;
    }

    private static String kind(Item control, Item label) {
        Item item = control == null ? label : control;
        if (item.radio) return "radio";
        if (item.checkbox) return "checkbox";
        if (item.checkable) return "switch";
        if (item.editable) return "text field";
        if (item.slider) return "slider";
        if (item.link) return "link";
        if (item.button) return "button";
        if (item.scrollable) return "scroll area";
        if (item.selected && item.role.toLowerCase(Locale.ROOT).contains("tab")) return "tab";
        return control == null ? "" : "control";
    }

    private static String state(Item control, boolean selected) {
        if (control == null) return selected ? "selected" : "";
        if (control.radio || control.checkbox || control.checkable) return control.checked ? "checked" : "unchecked";
        return selected ? "selected" : "";
    }

    private static String join(List<Item> items, int start) {
        StringBuilder out = new StringBuilder();
        for (int i = start; i < items.size(); i++) {
            if (out.length() > 0) out.append(", ");
            out.append(items.get(i).label);
        }
        return out.toString();
    }

    private static String clean(String value) {
        if (value == null) return "";
        return value.trim().replaceAll("\\s+", " ");
    }
}
