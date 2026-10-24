package dev.codex.aubridge;

/**
 * One approval a person answers on the phone. The host starts it with the text and the exact
 * command that will run, then polls {@link #current}; nothing here is reachable from the
 * accessibility tree, so the agent cannot answer it.
 */
final class Approval {
    static final int CAPABILITY = 16;
    static final long MIN_MS = 5_000;
    static final long MAX_MS = 300_000;

    private static String state = "none";
    private static long deadline;
    private static String text = "";
    private static String detail = "";

    private Approval() {}

    /** Starts a prompt unless one is already waiting; false when one is. */
    static synchronized boolean start(String prompt, String command, long timeoutMs, long now) {
        if ("pending".equals(current(now))) return false;
        state = "pending";
        text = clip(prompt, 300);
        detail = clip(command, 300);
        deadline = now + Math.max(MIN_MS, Math.min(MAX_MS, timeoutMs));
        return true;
    }

    /** "none", "pending", "allow", "deny" or "timeout". */
    static synchronized String current(long now) {
        if ("pending".equals(state) && now >= deadline) state = "timeout";
        return state;
    }

    /** A person's answer; ignored unless a prompt is still waiting. */
    static synchronized void answer(boolean allow, long now) {
        if ("pending".equals(current(now))) state = allow ? "allow" : "deny";
    }

    static synchronized boolean waiting(long now) {
        return "pending".equals(current(now));
    }

    static synchronized String text() {
        return text;
    }

    static synchronized String detail() {
        return detail;
    }

    /** The host has read the answer. */
    static synchronized void clear() {
        state = "none";
    }

    private static String clip(String value, int max) {
        String v = value == null ? "" : value;
        return v.length() <= max ? v : v.substring(0, max);
    }
}
