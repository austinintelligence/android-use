package dev.codex.aubridge;

import java.util.concurrent.atomic.AtomicInteger;

/** Capture rules free of Android types, so the JVM can test them. */
final class Safety {
    private Safety() {}

    /** True when the first {@code n} bytes of {@code pcm} are all zero (a muted microphone). */
    static boolean silent(byte[] pcm, int n) {
        for (int i = 0; i < n; i++) if (pcm[i] != 0) return false;
        return true;
    }

    /** Lets a callback that fires after the caller gave up release its resource itself. */
    static final class Abandon {
        private boolean gone;

        /** Runs {@code keep} while the caller still waits, else {@code close}. */
        synchronized void offer(Runnable keep, Runnable close) {
            (gone ? close : keep).run();
        }

        synchronized void abandon() {
            gone = true;
        }
    }

    /** Counts providers still able to answer, so a disabled provider cannot stall a read. */
    static final class Providers {
        private final AtomicInteger live;

        Providers(int count) {
            live = new AtomicInteger(count);
        }

        /** Marks one provider lost; true when none is left. */
        boolean lost() {
            return live.decrementAndGet() <= 0;
        }

        boolean none() {
            return live.get() <= 0;
        }
    }
}
