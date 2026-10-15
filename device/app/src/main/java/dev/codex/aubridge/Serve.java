package dev.codex.aubridge;

import java.io.Closeable;
import java.io.IOException;
import java.util.function.BooleanSupplier;

/** The accept loop, kept free of Android types so a JVM test can drive it. */
final class Serve {
    interface Source<C extends Closeable> {
        C next() throws IOException;
    }

    interface Handler<C> {
        void handle(C connection) throws Exception;
    }

    interface Opener<S> {
        S open() throws IOException;
    }

    private Serve() {}

    /** Opens a server socket, retrying with doubling delays; throws the last failure. */
    static <S> S bind(Opener<S> opener, int attempts, long firstDelayMs) throws IOException {
        long delay = firstDelayMs;
        for (int i = 1; ; i++) {
            try {
                return opener.open();
            } catch (IOException e) {
                if (i >= attempts) throw e;
            }
            try {
                Thread.sleep(delay);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                throw new IOException("interrupted while binding");
            }
            delay *= 2;
        }
    }

    /** Serves connections until closed; one connection's failure closes only that connection. */
    static <C extends Closeable> void loop(
            Source<C> source, Handler<C> handler, BooleanSupplier open) {
        while (open.getAsBoolean()) {
            C connection;
            try {
                connection = source.next();
            } catch (IOException e) {
                if (!open.getAsBoolean()) return;
                pause();
                continue;
            }
            try {
                handler.handle(connection);
            } catch (Exception e) {
                try {
                    connection.close();
                } catch (IOException ignored) {
                }
            }
        }
    }

    private static void pause() {
        try {
            Thread.sleep(50);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
        }
    }
}
