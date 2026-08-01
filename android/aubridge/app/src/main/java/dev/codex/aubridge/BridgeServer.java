package dev.codex.aubridge;

import android.net.LocalServerSocket;
import android.net.LocalSocket;
import android.util.Log;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.InputStream;
import java.io.OutputStream;
import java.nio.charset.StandardCharsets;
import java.util.concurrent.ArrayBlockingQueue;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.RejectedExecutionException;
import java.util.concurrent.ThreadPoolExecutor;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;

final class BridgeServer {
    private static final String SOCKET_NAME = "codex_au_bridge";
    private static final String TAG = "AUBridge";
    private final BridgeService service;
    private final SessionAuth auth;
    private final AtomicBoolean running = new AtomicBoolean(true);
    // Media and malformed-client pressure must not create an unbounded number
    // of Android threads.  The queue is deliberately finite; rejected clients
    // are closed by the accept loop instead of growing memory indefinitely.
    private final ExecutorService clients = new ThreadPoolExecutor(
            2,
            8,
            30L,
            TimeUnit.SECONDS,
            new ArrayBlockingQueue<>(32),
            new ThreadPoolExecutor.AbortPolicy());
    private LocalServerSocket socket;
    private Thread acceptThread;

    BridgeServer(BridgeService service, String token) {
        this.service = service;
        this.auth = new SessionAuth(token);
    }

    void start() {
        // Keep accept separate from client workers. A persistent USB session
        // must not consume the only worker that would otherwise service a
        // concurrent Wi-Fi/mDNS session for the same helper socket.
        acceptThread = new Thread(() -> {
            try {
                LocalServerSocket bound = null;
                for (int attempt = 0; attempt < 20 && running.get(); attempt++) {
                    try {
                        bound = new LocalServerSocket(SOCKET_NAME);
                        break;
                    } catch (Exception bindError) {
                        if (attempt == 19) {
                            Log.e(TAG, "unable to bind helper socket after bounded retries", bindError);
                            return;
                        }
                        try {
                            Thread.sleep(100L);
                        } catch (InterruptedException interrupted) {
                            Thread.currentThread().interrupt();
                            return;
                        }
                    }
                }
                if (bound == null || !running.get()) {
                    if (bound != null) bound.close();
                    return;
                }
                socket = bound;
                while (running.get()) {
                    LocalSocket client = socket.accept();
                    try {
                        clients.execute(() -> handleClient(client));
                    } catch (RejectedExecutionException rejected) {
                        try {
                            client.close();
                        } catch (Exception ignored) {
                        }
                    }
                }
            } catch (Exception error) {
                if (running.get()) {
                    Log.e(TAG, "helper accept loop stopped", error);
                }
            }
        }, "au-bridge-accept");
        acceptThread.start();
    }

    void close() {
        running.set(false);
        try {
            if (socket != null) {
                socket.close();
            }
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
        clients.shutdownNow();
    }

    private void handleClient(LocalSocket client) {
        try (LocalSocket ignored = client; InputStream input = client.getInputStream(); OutputStream output = client.getOutputStream()) {
            RequestSequence sequence = new RequestSequence();
            while (running.get()) {
                JSONObject request;
                try {
                    request = new JSONObject(new String(FrameCodec.readFrame(input), StandardCharsets.UTF_8));
                } catch (java.io.EOFException end) {
                    return;
                }
                JSONObject response = handle(request, sequence);
                FrameCodec.writeFrame(output, response.toString().getBytes(StandardCharsets.UTF_8));
            }
        } catch (Exception ignored) {
        }
    }

    private JSONObject handle(JSONObject request, RequestSequence sequence) {
        long id = request.optLong("id", 0L);
        try {
            if (request.optInt("version", 0) != 1) {
                return error(id, "E_PROTOCOL", "unsupported helper protocol version");
            }
            try {
                auth.authenticate(request.optString("token", ""), request.optString("nonce", ""));
            } catch (SessionAuth.AuthError authentication) {
                return error(id, authentication.code, authentication.getMessage());
            }
            sequence.accept(request.optLong("sequence", 0L));
            service.heartbeat();
            String operation = request.optString("operation", "");
            JSONObject args = request.optJSONObject("args");
            if (args == null) {
                args = new JSONObject();
            }
            JSONObject data = dispatch(operation, args);
            return ok(id, data);
        } catch (BridgeError error) {
            return error(id, error.code, error.getMessage());
        } catch (Exception error) {
            return error(id, "E_HELPER", error.getClass().getSimpleName() + ": " + error.getMessage());
        }
    }

    private JSONObject dispatch(String operation, JSONObject args) throws Exception {
        if ("heartbeat".equals(operation)) {
            return new JSONObject().put("heartbeat", true);
        }
        if (operation.startsWith("ui.")) {
            AubridgeAccessibilityService accessibility = AubridgeAccessibilityService.current();
            if (accessibility == null) {
                throw new BridgeError("E_CAPABILITY", "Accessibility service is not enabled");
            }
            return accessibility.handle(operation, args);
        }
        if (operation.startsWith("notification.")) {
            AubridgeNotificationListener listener = AubridgeNotificationListener.current();
            if (listener == null) {
                throw new BridgeError("E_CAPABILITY", "Notification access is not enabled");
            }
            return listener.handle(operation, args);
        }
        if (operation.equals("camera.list")) {
            return CameraCapture.list(service);
        }
        if (operation.equals("camera.snapshot")) {
            return CameraCapture.snapshot(service, args.optString("camera", ""), service::heartbeatFresh);
        }
        if (operation.equals("camera.record")) {
            return CameraCapture.record(service, args.optString("camera", ""), args.optInt("seconds", 3), service::heartbeatFresh);
        }
        if (operation.equals("camera.mjpeg")) {
            return CameraCapture.mjpeg(service, args.optString("camera", ""), args.optInt("seconds", 3), service::heartbeatFresh);
        }
        if (operation.equals("microphone.capture")) {
            return AudioCapture.capture(service, args.optInt("seconds", 3), service::heartbeatFresh);
        }
        if (operation.equals("microphone.pcm")) {
            return AudioCapture.pcm(service, args.optInt("seconds", 3), service::heartbeatFresh);
        }
        if (operation.startsWith("location.")) {
            return LocationControl.handle(service, operation, args);
        }
        throw new BridgeError("E_ARGS", "unknown helper operation " + operation);
    }

    private static JSONObject ok(long id, JSONObject data) {
        try {
            return new JSONObject().put("version", 1).put("id", id).put("ok", true).put("data", data);
        } catch (Exception error) {
            throw new IllegalStateException("cannot encode helper success response", error);
        }
    }

    private static JSONObject error(long id, String code, String message) {
        try {
            return new JSONObject().put("version", 1).put("id", id).put("ok", false).put("code", code).put("message", message == null ? "" : message);
        } catch (Exception error) {
            throw new IllegalStateException("cannot encode helper error response", error);
        }
    }

    static final class BridgeError extends Exception {
        final String code;

        BridgeError(String code, String message) {
            super(message);
            this.code = code;
        }
    }

    /** Monotonic request ordering for one authenticated local-socket client. */
    static final class RequestSequence {
        private long expected = 1L;

        void accept(long received) throws BridgeError {
            if (received != expected || expected == Long.MAX_VALUE) {
                throw new BridgeError("E_AUTH", "invalid helper request sequence");
            }
            expected++;
        }

        long expected() {
            return expected;
        }
    }
}
