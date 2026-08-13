package dev.codex.aubridge;

import android.annotation.SuppressLint;
import android.app.Notification;
import android.app.Service;
import android.content.pm.ServiceInfo;
import android.os.Build;

/**
 * Keeps the bridge's foreground-service type as narrow as possible.
 *
 * <p>The long-running core uses {@code specialUse} on Android 14 and newer.
 * Camera and microphone types are added only while a command is actively using
 * that sensor. Calls are serialized and reference counted because BridgeServer
 * intentionally serves several authenticated clients concurrently.</p>
 */
@SuppressLint("NewApi") // Calls are guarded by the injected/runtime SDK level below.
final class ForegroundServiceTypes {
    private static final int NOTIFICATION_ID = 4101;

    interface Starter {
        void startUntyped();

        void startTyped(int types);
    }

    private final int sdkInt;
    private final Starter starter;
    private int cameraReferences;
    private int microphoneReferences;

    static ForegroundServiceTypes forService(Service service, Notification notification) {
        return new ForegroundServiceTypes(Build.VERSION.SDK_INT, new Starter() {
            @Override
            public void startUntyped() {
                service.startForeground(NOTIFICATION_ID, notification);
            }

            @Override
            public void startTyped(int types) {
                service.startForeground(NOTIFICATION_ID, notification, types);
            }
        });
    }

    ForegroundServiceTypes(int sdkInt, Starter starter) {
        this.sdkInt = sdkInt;
        this.starter = starter;
    }

    synchronized void startCore() {
        try {
            applyCurrentTypes();
        } catch (RuntimeException error) {
            throw new IllegalStateException(
                    "Cannot promote the AU Bridge core foreground service: "
                            + error.getClass().getSimpleName(),
                    error);
        }
    }

    Lease acquireCamera() throws BridgeServer.BridgeError {
        return acquire(Sensor.CAMERA);
    }

    Lease acquireMicrophone() throws BridgeServer.BridgeError {
        return acquire(Sensor.MICROPHONE);
    }

    private synchronized Lease acquire(Sensor sensor) throws BridgeServer.BridgeError {
        increment(sensor);
        try {
            applyCurrentTypes();
        } catch (RuntimeException error) {
            decrement(sensor);
            throw transitionError(sensor, false, error);
        }
        return new Lease(this, sensor);
    }

    private synchronized void release(Sensor sensor) throws BridgeServer.BridgeError {
        decrement(sensor);
        try {
            applyCurrentTypes();
        } catch (RuntimeException error) {
            // Keep the tracked reference aligned with the type Android is still
            // running. Lease.close() remains retryable until this transition
            // succeeds, so a transient policy failure cannot silently leak an
            // untracked camera or microphone foreground type.
            increment(sensor);
            throw transitionError(sensor, true, error);
        }
    }

    private void applyCurrentTypes() {
        int types = 0;
        if (sdkInt >= 34) {
            types |= ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE;
        }
        if (sdkInt >= 29 && cameraReferences > 0) {
            types |= ServiceInfo.FOREGROUND_SERVICE_TYPE_CAMERA;
        }
        if (sdkInt >= 30 && microphoneReferences > 0) {
            types |= ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE;
        }

        // The two-argument overload means "all manifest types" on API 29 and
        // newer. Use the typed overload even for NONE so an idle API 29-33
        // bridge does not accidentally retain camera and microphone types.
        if (sdkInt >= 29) {
            starter.startTyped(types);
        } else {
            starter.startUntyped();
        }
    }

    private void increment(Sensor sensor) {
        if (sensor == Sensor.CAMERA) {
            cameraReferences++;
        } else {
            microphoneReferences++;
        }
    }

    private void decrement(Sensor sensor) {
        if (sensor == Sensor.CAMERA) {
            if (cameraReferences <= 0) {
                throw new IllegalStateException("camera foreground-service reference underflow");
            }
            cameraReferences--;
        } else {
            if (microphoneReferences <= 0) {
                throw new IllegalStateException("microphone foreground-service reference underflow");
            }
            microphoneReferences--;
        }
    }

    private static BridgeServer.BridgeError transitionError(
            Sensor sensor,
            boolean releasing,
            RuntimeException error) {
        boolean expectedPolicyFailure = isExpectedPolicyFailure(error);
        String code = expectedPolicyFailure ? "E_PERMISSION" : "E_HELPER";
        String action = releasing
                ? "release the " + sensor.label + " foreground-service type after the command"
                : "enable the " + sensor.label + " foreground-service type";
        String guidance = expectedPolicyFailure
                ? "; open AU Bridge while the device is unlocked and grant the requested permission"
                : "; the helper manifest or foreground-service state is inconsistent";
        return new BridgeServer.BridgeError(
                code,
                "Android could not " + action + guidance
                        + " (" + error.getClass().getSimpleName() + ")");
    }

    private static boolean isExpectedPolicyFailure(RuntimeException error) {
        if (error instanceof SecurityException) {
            return true;
        }
        // These exception classes are newer than the API-26 floor. Match their
        // hierarchy by name so loading this class never resolves a newer type
        // on an older runtime.
        for (Class<?> type = error.getClass(); type != null; type = type.getSuperclass()) {
            String name = type.getSimpleName();
            if ("ForegroundServiceStartNotAllowedException".equals(name)
                    || "ForegroundServiceTypeNotAllowedException".equals(name)) {
                return true;
            }
        }
        return false;
    }

    enum Sensor {
        CAMERA("camera"),
        MICROPHONE("microphone");

        final String label;

        Sensor(String label) {
            this.label = label;
        }
    }

    static final class Lease implements AutoCloseable {
        private final ForegroundServiceTypes owner;
        private final Sensor sensor;
        private boolean closed;

        Lease(ForegroundServiceTypes owner, Sensor sensor) {
            this.owner = owner;
            this.sensor = sensor;
        }

        @Override
        public void close() throws BridgeServer.BridgeError {
            synchronized (this) {
                if (closed) {
                    return;
                }
                // Mark closed only after the owner has successfully applied the
                // reduced type set. If Android rejects demotion, owner.release
                // restores the counter and this same lease can be retried.
                owner.release(sensor);
                closed = true;
            }
        }
    }
}
