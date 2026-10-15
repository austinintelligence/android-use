package dev.codex.aubridge;

/** Android 14+ screen-recording rules, kept free of Android types so the JVM can test them. */
final class Recording {
    /** The order Android 14 enforces; each step needs the one before it. */
    enum Step {
        SERVICE_PROMOTED,
        PROJECTION_OBTAINED,
        CALLBACK_REGISTERED,
        DISPLAY_CREATED
    }

    private Step done;

    /** Records {@code step}, rejecting any that skips or repeats the required order. */
    void reach(Step step) {
        int expected = done == null ? 0 : done.ordinal() + 1;
        if (step.ordinal() != expected)
            throw new IllegalStateException(step + " out of order after " + done);
        done = step;
    }

    /** A consent result is single-use from Android 14 (API 34); older releases may reuse it. */
    static boolean tokenSingleUse(int sdk) {
        return sdk >= 34;
    }

    /** Holds one value and hands it out once when the platform makes it single-use. */
    static final class Token<T> {
        private T value;

        synchronized void set(T next) {
            value = next;
        }

        synchronized boolean present() {
            return value != null;
        }

        synchronized T take(int sdk) {
            T taken = value;
            if (tokenSingleUse(sdk)) value = null;
            return taken;
        }

        synchronized void clear() {
            value = null;
        }
    }
}
