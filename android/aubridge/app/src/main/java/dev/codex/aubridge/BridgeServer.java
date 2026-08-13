package dev.codex.aubridge;

import android.net.Credentials;
import android.net.LocalServerSocket;
import android.net.LocalSocket;
import android.util.Log;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.InputStream;
import java.io.OutputStream;
import java.nio.charset.StandardCharsets;
import java.util.Set;
import java.util.concurrent.ArrayBlockingQueue;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.RejectedExecutionException;
import java.util.concurrent.ThreadPoolExecutor;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;

final class BridgeServer {
    private static final String SOCKET_NAME = "codex_au_bridge";
    private static final String TAG = "AUBridge";
    private static final int CLIENT_READ_TIMEOUT_MS = 10_000;
    private final BridgeService service;
    private final SessionAuth auth;
    private final Runnable onUnexpectedStop;
    private final AtomicBoolean running = new AtomicBoolean(false);
    private final AtomicBoolean started = new AtomicBoolean(false);
    private final AtomicBoolean closed = new AtomicBoolean(false);
    private final AtomicBoolean listening = new AtomicBoolean(false);
    private final CountDownLatch boundLatch = new CountDownLatch(1);
    private final Set<LocalSocket> activeClients = ConcurrentHashMap.newKeySet();
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
    private volatile LocalServerSocket socket;
    private volatile Thread acceptThread;
    private volatile Throwable listenerFailure;

    BridgeServer(BridgeService service, SessionAuth auth, Runnable onUnexpectedStop) {
        this.service = service;
        this.auth = auth;
        this.onUnexpectedStop = onUnexpectedStop;
    }

    void start() {
        if (closed.get() || !started.compareAndSet(false, true)) {
            throw new IllegalStateException("helper listener can only be started once");
        }
        running.set(true);
        // Keep accept separate from client workers. A persistent USB session
        // must not consume the only worker that would otherwise service a
        // concurrent Wi-Fi/mDNS session for the same helper socket.
        acceptThread = new Thread(this::runAcceptLoop, "au-bridge-accept");
        acceptThread.start();
    }

    boolean awaitListening(long timeoutMs) throws InterruptedException {
        if (!started.get()) {
            throw new IllegalStateException("helper listener has not been started");
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
                activeClients.add(client);
                try {
                    clients.execute(() -> handleClient(client));
                } catch (RejectedExecutionException rejected) {
                    activeClients.remove(client);
                    try {
                        client.close();
                    } catch (Exception ignored) {
                    }
                }
            }
        } catch (Exception error) {
            if (running.get()) {
                listenerFailure = error;
                Log.e(TAG, "helper accept loop stopped", error);
            }
        } finally {
            listening.set(false);
            boundLatch.countDown();
            try {
                if (bound != null) bound.close();
            } catch (Exception ignored) {
            }
            socket = null;
            if (running.getAndSet(false)) {
                notifyUnexpectedStop();
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

    private void notifyUnexpectedStop() {
        try {
            onUnexpectedStop.run();
        } catch (RuntimeException callbackFailure) {
            Log.e(TAG, "helper listener failure callback stopped", callbackFailure);
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
            if (current != null) {
                current.close();
            }
        } catch (Exception ignored) {
        }
        for (LocalSocket client : activeClients) {
            try {
                client.close();
            } catch (Exception ignored) {
            }
        }
        activeClients.clear();
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
        try (LocalSocket ignored = client) {
            Credentials credentials = client.getPeerCredentials();
            if (credentials == null || !BootstrapServer.isAuthorizedUid(credentials.getUid())) {
                return;
            }
            // A client that connects and sends only a partial frame must not
            // pin one of the bounded workers forever. Long-running operations
            // happen after a complete frame is read, so this idle read limit
            // does not cap legitimate media or plan execution time.
            client.setSoTimeout(CLIENT_READ_TIMEOUT_MS);
            try (InputStream input = client.getInputStream(); OutputStream output = client.getOutputStream()) {
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
                if (!response.optBoolean("ok", false)
                        && "E_AUTH".equals(response.optString("code", ""))) {
                    return;
                }
            }
            }
        } catch (Exception ignored) {
        } finally {
            activeClients.remove(client);
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
        if ("plan.run".equals(operation)) {
            AubridgeAccessibilityService accessibility = AubridgeAccessibilityService.current();
            if (accessibility == null) {
                throw new BridgeError("E_CAPABILITY", "Accessibility service is not enabled");
            }
            return PlanExecutor.execute(accessibility, args);
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
            return withCameraForeground(() -> {
                try {
                    return CameraCapture.snapshot(service, args.optString("camera", ""), service::heartbeatFresh);
                } catch (SecurityException error) {
                    throw sensorPermissionError("camera", error);
                }
            });
        }
        if (operation.equals("camera.record")) {
            return withCameraForeground(() -> {
                try {
                    return CameraCapture.record(service, args.optString("camera", ""), args.optInt("seconds", 3), service::heartbeatFresh);
                } catch (SecurityException error) {
                    throw sensorPermissionError("camera", error);
                }
            });
        }
        if (operation.equals("camera.mjpeg")) {
            return withCameraForeground(() -> {
                try {
                    return CameraCapture.mjpeg(service, args.optString("camera", ""), args.optInt("seconds", 3), service::heartbeatFresh);
                } catch (SecurityException error) {
                    throw sensorPermissionError("camera", error);
                }
            });
        }
        if (operation.equals("microphone.capture")) {
            return withMicrophoneForeground(() -> {
                try {
                    return AudioCapture.capture(service, args.optInt("seconds", 3), service::heartbeatFresh);
                } catch (SecurityException error) {
                    throw sensorPermissionError("microphone", error);
                }
            });
        }
        if (operation.equals("microphone.pcm")) {
            return withMicrophoneForeground(() -> {
                try {
                    return AudioCapture.pcm(service, args.optInt("seconds", 3), service::heartbeatFresh);
                } catch (SecurityException error) {
                    throw sensorPermissionError("microphone", error);
                }
            });
        }
        if (operation.startsWith("artifact.")) {
            return PrivateArtifacts.handle(service, operation, args);
        }
        if (operation.startsWith("location.")) {
            return LocationControl.handle(service, operation, args);
        }
        throw new BridgeError("E_ARGS", "unknown helper operation " + operation);
    }

    private JSONObject withCameraForeground(SensorCommand command) throws Exception {
        return withSensorForeground(service.beginCameraForeground(), command);
    }

    private JSONObject withMicrophoneForeground(SensorCommand command) throws Exception {
        return withSensorForeground(service.beginMicrophoneForeground(), command);
    }

    private static JSONObject withSensorForeground(
            ForegroundServiceTypes.Lease lease,
            SensorCommand command) throws Exception {
        Throwable actionFailure = null;
        try {
            return command.run();
        } catch (Exception error) {
            actionFailure = error;
            throw error;
        } catch (Error error) {
            actionFailure = error;
            throw error;
        } finally {
            try {
                closeLeaseWithRetry(lease);
            } catch (BridgeError cleanupFailure) {
                if (actionFailure != null) {
                    actionFailure.addSuppressed(cleanupFailure);
                } else {
                    BridgeError reported = new BridgeError(
                            cleanupFailure.code,
                            "Sensor action completed, but foreground-service cleanup failed after one retry: "
                                    + cleanupFailure.getMessage()
                                    + "; stop and restart AU Bridge to reconcile the retained type");
                    reported.addSuppressed(cleanupFailure);
                    throw reported;
                }
            }
        }
    }

    private static void closeLeaseWithRetry(ForegroundServiceTypes.Lease lease)
            throws BridgeError {
        BridgeError firstFailure;
        try {
            lease.close();
            return;
        } catch (BridgeError error) {
            firstFailure = error;
        }
        try {
            lease.close();
        } catch (BridgeError secondFailure) {
            secondFailure.addSuppressed(firstFailure);
            throw secondFailure;
        }
    }

    private interface SensorCommand {
        JSONObject run() throws Exception;
    }

    private static BridgeError sensorPermissionError(String sensor, SecurityException error) {
        return new BridgeError(
                "E_PERMISSION",
                "Android denied " + sensor
                        + " access; open AU Bridge while the device is unlocked and grant the requested permission ("
                        + error.getClass().getSimpleName() + ")");
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
