package dev.codex.aubridge;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.Service;
import android.content.Intent;
import android.os.IBinder;
import android.os.SystemClock;

import java.util.concurrent.atomic.AtomicLong;

/** Shared implementation for the current AU foreground bridge component. */
public class BridgeService extends Service {
    private static final long LISTENER_START_TIMEOUT_MS = 3_000L;
    private static volatile boolean ready;
    private final Object lifecycleLock = new Object();
    private final AtomicLong heartbeatAtMs = new AtomicLong(0L);
    private BridgeServer server;
    private ForegroundServiceTypes foregroundServiceTypes;
    private BootstrapServer bootstrapServer;
    private boolean shuttingDown;

    @Override
    public void onCreate() {
        super.onCreate();
        synchronized (lifecycleLock) {
            ready = false;
            shuttingDown = false;
        }
        try {
            // Version 2 used a persistent app-file bearer token. SessionAuth
            // now mints short-lived credentials through the shell/root-gated
            // bootstrap socket, so old credentials must not survive migration.
            removeLegacyCredentialFile("bridge_token");
            removeLegacyCredentialFile("bridge_auth_version");
            SessionAuth auth = new SessionAuth();
            createChannel();
            Notification notification = new Notification.Builder(this, "au-bridge")
                    .setSmallIcon(android.R.drawable.ic_dialog_info)
                    .setContentTitle("AU Bridge active")
                    .setContentText("Authenticated local ADB-forwarded control only")
                    .build();
            foregroundServiceTypes = ForegroundServiceTypes.forService(this, notification);
            foregroundServiceTypes.startCore();

            server = new BridgeServer(this, auth, this::listenerStoppedUnexpectedly);
            bootstrapServer = new BootstrapServer(auth, this::listenerStoppedUnexpectedly);
            server.start();
            bootstrapServer.start();
            awaitListeners();

            synchronized (lifecycleLock) {
                if (shuttingDown || !server.isListening() || !bootstrapServer.isListening()) {
                    throw new IllegalStateException("authenticated helper listeners stopped during startup");
                }
                ready = true;
            }
        } catch (InterruptedException interrupted) {
            Thread.currentThread().interrupt();
            failStartup();
            throw new IllegalStateException("Interrupted while starting authenticated helper listeners", interrupted);
        } catch (RuntimeException error) {
            failStartup();
            throw error;
        }
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        return START_STICKY;
    }

    @Override
    public void onDestroy() {
        synchronized (lifecycleLock) {
            shuttingDown = true;
            ready = false;
        }
        heartbeatAtMs.set(0L);
        closeListeners();
        super.onDestroy();
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }

    static boolean isRunning() {
        return ready;
    }

    public void heartbeat() {
        heartbeatAtMs.set(System.currentTimeMillis());
    }

    public boolean heartbeatFresh() {
        long value = heartbeatAtMs.get();
        return value > 0 && System.currentTimeMillis() - value <= 3_000L;
    }

    ForegroundServiceTypes.Lease beginCameraForeground() throws BridgeServer.BridgeError {
        if (!ready || foregroundServiceTypes == null) {
            throw new BridgeServer.BridgeError("E_HELPER", "foreground service is not ready");
        }
        return foregroundServiceTypes.acquireCamera();
    }

    ForegroundServiceTypes.Lease beginMicrophoneForeground() throws BridgeServer.BridgeError {
        if (!ready || foregroundServiceTypes == null) {
            throw new BridgeServer.BridgeError("E_HELPER", "foreground service is not ready");
        }
        return foregroundServiceTypes.acquireMicrophone();
    }

    private void awaitListeners() throws InterruptedException {
        long deadline = SystemClock.elapsedRealtime() + LISTENER_START_TIMEOUT_MS;
        if (!server.awaitListening(remainingStartupMs(deadline))) {
            throw new IllegalStateException(
                    "Cannot bind authenticated helper command listener: "
                            + server.failureDescription());
        }
        if (!bootstrapServer.awaitListening(remainingStartupMs(deadline))) {
            throw new IllegalStateException(
                    "Cannot bind authenticated helper bootstrap listener: "
                            + bootstrapServer.failureDescription());
        }
    }

    private static long remainingStartupMs(long deadline) {
        return Math.max(0L, deadline - SystemClock.elapsedRealtime());
    }

    private void listenerStoppedUnexpectedly() {
        boolean shouldStop;
        synchronized (lifecycleLock) {
            ready = false;
            shouldStop = !shuttingDown;
        }
        if (shouldStop) {
            stopSelf();
        }
    }

    private void failStartup() {
        synchronized (lifecycleLock) {
            ready = false;
            shuttingDown = true;
        }
        closeListeners();
    }

    private void closeListeners() {
        BridgeServer command = server;
        if (command != null) {
            command.close();
        }
        BootstrapServer bootstrap = bootstrapServer;
        if (bootstrap != null) {
            bootstrap.close();
        }
    }

    private void removeLegacyCredentialFile(String name) {
        try {
            super.deleteFile(name);
        } catch (RuntimeException ignored) {
            // Old credentials are defense-in-depth only; listener startup
            // remains independent of deleting already-private migration data.
        }
    }

    private void createChannel() {
        NotificationManager manager = getSystemService(NotificationManager.class);
        manager.createNotificationChannel(new NotificationChannel("au-bridge", "AU Bridge", NotificationManager.IMPORTANCE_LOW));
    }
}
