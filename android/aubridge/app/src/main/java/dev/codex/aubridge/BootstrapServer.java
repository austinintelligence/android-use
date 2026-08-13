package dev.codex.aubridge;

import android.net.Credentials;
import android.net.LocalServerSocket;
import android.net.LocalSocket;
import android.util.Log;

import org.json.JSONObject;

import java.io.InputStream;
import java.io.OutputStream;
import java.nio.charset.StandardCharsets;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;

/**
 * One-purpose credential bootstrap for a non-debuggable helper.
 *
 * The socket is reachable from the host only through an authorized ADB
 * localabstract forward. Android reports the device-side peer as shell/root;
 * ordinary applications are rejected before any request bytes are read.
 */
final class BootstrapServer {
    static final String SOCKET_NAME = "codex_au_bridge_bootstrap";
    private static final String TAG = "AUBridgeBootstrap";
    private static final int ROOT_UID = 0;
    private static final int SHELL_UID = 2000;
    private static final int MIN_NONCE_BYTES = 12;
    private static final int MAX_NONCE_BYTES = 128;

    private final SessionAuth auth;
    private final Runnable onUnexpectedStop;
    private final AtomicBoolean running = new AtomicBoolean(false);
    private final AtomicBoolean started = new AtomicBoolean(false);
    private final AtomicBoolean closed = new AtomicBoolean(false);
    private final AtomicBoolean listening = new AtomicBoolean(false);
    private final CountDownLatch boundLatch = new CountDownLatch(1);
    private volatile LocalServerSocket socket;
    private volatile LocalSocket activeClient;
    private volatile Thread acceptThread;
    private volatile Throwable listenerFailure;

    BootstrapServer(SessionAuth auth, Runnable onUnexpectedStop) {
        this.auth = auth;
        this.onUnexpectedStop = onUnexpectedStop;
    }

    void start() {
        if (closed.get() || !started.compareAndSet(false, true)) {
            throw new IllegalStateException("bootstrap listener can only be started once");
        }
        running.set(true);
        acceptThread = new Thread(this::runAcceptLoop, "au-bootstrap-accept");
        acceptThread.start();
    }

    boolean awaitListening(long timeoutMs) throws InterruptedException {
        if (!started.get()) {
            throw new IllegalStateException("bootstrap listener has not been started");
        }
        return boundLatch.await(Math.max(0L, timeoutMs), TimeUnit.MILLISECONDS)
                && listening.get();
    }

    boolean isListening() {
        return listening.get();
    }

    String failureDescription() {
        Throwable failure = listenerFailure;
        if (failure == null) {
            return "listener did not bind before the startup deadline";
        }
        String message = failure.getMessage();
        if (message == null || message.trim().isEmpty()) {
            return failure.getClass().getSimpleName();
        }
        return failure.getClass().getSimpleName() + ": "
                + message.replace('\r', ' ').replace('\n', ' ');
    }

    private void runAcceptLoop() {
        LocalServerSocket bound = null;
        try {
            bound = bindWithRetry();
            if (bound == null || !running.get()) {
                return;
            }
            socket = bound;
            listening.set(true);
            boundLatch.countDown();
            while (running.get()) {
                LocalSocket client = bound.accept();
                activeClient = client;
                try {
                    handle(client);
                } finally {
                    activeClient = null;
                }
            }
        } catch (Exception error) {
            if (running.get()) {
                listenerFailure = error;
                // Never include request, nonce, credential, or token data in
                // bootstrap logs. The exception class is enough for lifecycle
                // diagnosis and failureDescription() is host-internal only.
                Log.e(TAG, "bootstrap accept loop stopped: "
                        + error.getClass().getSimpleName());
            }
        } finally {
            listening.set(false);
            boundLatch.countDown();
            try {
                if (bound != null) bound.close();
            } catch (Exception ignored) {
            }
            socket = null;
            activeClient = null;
            if (running.getAndSet(false)) {
                notifyUnexpectedStop();
            }
        }
    }

    private void notifyUnexpectedStop() {
        try {
            onUnexpectedStop.run();
        } catch (RuntimeException callbackFailure) {
            Log.e(TAG, "bootstrap listener failure callback stopped: "
                    + callbackFailure.getClass().getSimpleName());
        }
    }

    void close() {
        if (!closed.compareAndSet(false, true)) {
            return;
        }
        running.set(false);
        listening.set(false);
        boundLatch.countDown();
        try {
            LocalServerSocket current = socket;
            if (current != null) current.close();
        } catch (Exception ignored) {
        }
        try {
            LocalSocket client = activeClient;
            if (client != null) client.close();
        } catch (Exception ignored) {
        }
        if (acceptThread != null) {
            acceptThread.interrupt();
            if (Thread.currentThread() != acceptThread) {
                try {
                    acceptThread.join(500L);
                } catch (InterruptedException interrupted) {
                    Thread.currentThread().interrupt();
                }
            }
        }
    }

    private LocalServerSocket bindWithRetry() throws Exception {
        for (int attempt = 0; attempt < 20 && running.get(); attempt++) {
            try {
                return new LocalServerSocket(SOCKET_NAME);
            } catch (Exception bindError) {
                if (attempt == 19) throw bindError;
                Thread.sleep(100L);
            }
        }
        return null;
    }

    private void handle(LocalSocket client) {
        try (LocalSocket ignored = client) {
            Credentials credentials = client.getPeerCredentials();
            if (credentials == null || !isAuthorizedUid(credentials.getUid())) {
                return;
            }
            client.setSoTimeout(2_000);
            try (InputStream input = client.getInputStream(); OutputStream output = client.getOutputStream()) {
                JSONObject request = new JSONObject(new String(FrameCodec.readFrame(input), StandardCharsets.UTF_8));
                String nonce = request.optString("nonce", "");
                if (request.optInt("version", 0) != 1
                        || !"bootstrap".equals(request.optString("operation", ""))
                        || !isValidNonce(nonce)) {
                    FrameCodec.writeFrame(output, response(false, nonce, "", "E_AUTH").getBytes(StandardCharsets.UTF_8));
                    return;
                }
                FrameCodec.writeFrame(output, response(true, nonce, auth.issueSession(), "").getBytes(StandardCharsets.UTF_8));
            }
        } catch (Exception ignored) {
            // Bootstrap is fail-closed and intentionally does not log request,
            // nonce, credentials, or token material.
        }
    }

    static boolean isAuthorizedUid(int uid) {
        return uid == ROOT_UID || uid == SHELL_UID;
    }

    static boolean isValidNonce(String nonce) {
        int bytes = nonce.getBytes(StandardCharsets.UTF_8).length;
        return bytes >= MIN_NONCE_BYTES && bytes <= MAX_NONCE_BYTES;
    }

    private static String response(boolean ok, String nonce, String token, String code) throws Exception {
        JSONObject value = new JSONObject()
                .put("version", 1)
                .put("ok", ok)
                .put("nonce", nonce);
        if (ok) value.put("token", token);
        else value.put("code", code);
        return value.toString();
    }
}
