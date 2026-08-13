package dev.codex.aubridge;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;
import static org.junit.Assert.fail;

import android.content.pm.ServiceInfo;

import org.junit.Test;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.concurrent.ConcurrentLinkedQueue;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;

public final class ForegroundServiceTypesTest {
    @Test
    public void android14KeepsSpecialUseCoreAndScopesBothSensors() throws Exception {
        RecordingStarter starter = new RecordingStarter();
        ForegroundServiceTypes types = new ForegroundServiceTypes(34, starter);

        types.startCore();
        ForegroundServiceTypes.Lease camera = types.acquireCamera();
        ForegroundServiceTypes.Lease microphone = types.acquireMicrophone();
        camera.close();
        microphone.close();

        int core = ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE;
        assertEquals(Arrays.asList(
                core,
                core | ServiceInfo.FOREGROUND_SERVICE_TYPE_CAMERA,
                core | ServiceInfo.FOREGROUND_SERVICE_TYPE_CAMERA
                        | ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE,
                core | ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE,
                core), starter.typedCalls);
        assertEquals(0, starter.untypedCalls);
    }

    @Test
    public void preAndroid14CoreUsesExplicitNoneAndCameraIsTemporary() throws Exception {
        RecordingStarter starter = new RecordingStarter();
        ForegroundServiceTypes types = new ForegroundServiceTypes(33, starter);

        types.startCore();
        try (ForegroundServiceTypes.Lease ignored = types.acquireCamera()) {
            assertEquals(Arrays.asList(
                    0,
                    ServiceInfo.FOREGROUND_SERVICE_TYPE_CAMERA), starter.typedCalls);
        }

        assertEquals(Arrays.asList(
                0,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_CAMERA,
                0), starter.typedCalls);
        assertEquals(0, starter.untypedCalls);
    }

    @Test
    public void api29DoesNotRequestUnavailableMicrophoneType() throws Exception {
        RecordingStarter starter = new RecordingStarter();
        ForegroundServiceTypes types = new ForegroundServiceTypes(29, starter);

        types.startCore();
        try (ForegroundServiceTypes.Lease ignored = types.acquireMicrophone()) {
            // The microphone foreground-service type was added in API 30.
        }

        assertEquals(Arrays.asList(0, 0, 0), starter.typedCalls);
        assertEquals(0, starter.untypedCalls);
    }

    @Test
    public void api26UsesOnlyLegacyUntypedForegroundCalls() throws Exception {
        assertLegacyUntypedForegroundCalls(26);
    }

    @Test
    public void api28UsesOnlyLegacyUntypedForegroundCalls() throws Exception {
        assertLegacyUntypedForegroundCalls(28);
    }

    @Test
    public void sameSensorIsReferenceCountedAcrossConcurrentCommands() throws Exception {
        RecordingStarter starter = new RecordingStarter();
        ForegroundServiceTypes types = new ForegroundServiceTypes(34, starter);
        int core = ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE;
        int camera = core | ServiceInfo.FOREGROUND_SERVICE_TYPE_CAMERA;

        types.startCore();
        ForegroundServiceTypes.Lease first = types.acquireCamera();
        ForegroundServiceTypes.Lease second = types.acquireCamera();
        first.close();
        first.close();
        second.close();

        assertEquals(Arrays.asList(core, camera, camera, camera, core), starter.typedCalls);
    }

    @Test
    public void cameraReferencesAreSafeUnderTrueMultithreadedConcurrency() throws Exception {
        RecordingStarter starter = new RecordingStarter();
        ForegroundServiceTypes types = new ForegroundServiceTypes(34, starter);
        int core = ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE;
        int camera = core | ServiceInfo.FOREGROUND_SERVICE_TYPE_CAMERA;
        int workerCount = 8;
        CountDownLatch start = new CountDownLatch(1);
        CountDownLatch acquired = new CountDownLatch(workerCount);
        CountDownLatch release = new CountDownLatch(1);
        ConcurrentLinkedQueue<Throwable> failures = new ConcurrentLinkedQueue<>();
        List<Thread> workers = new ArrayList<>();

        types.startCore();
        for (int index = 0; index < workerCount; index++) {
            Thread worker = new Thread(() -> {
                boolean signaled = false;
                try {
                    start.await();
                    ForegroundServiceTypes.Lease lease = types.acquireCamera();
                    acquired.countDown();
                    signaled = true;
                    release.await();
                    lease.close();
                } catch (Throwable failure) {
                    failures.add(failure);
                    if (!signaled) acquired.countDown();
                }
            }, "fgs-camera-test-" + index);
            workers.add(worker);
            worker.start();
        }

        start.countDown();
        boolean allAcquired = acquired.await(5, TimeUnit.SECONDS);
        release.countDown();
        for (Thread worker : workers) {
            worker.join(5_000L);
            assertFalse("worker did not terminate", worker.isAlive());
        }
        assertTrue("all workers must hold a lease before release", allAcquired);
        assertTrue("concurrent lease failure: " + failures, failures.isEmpty());
        assertEquals(1 + workerCount * 2, starter.typedCalls.size());
        assertEquals(core, (int) starter.typedCalls.get(0));
        for (int index = 1; index < starter.typedCalls.size() - 1; index++) {
            assertEquals(camera, (int) starter.typedCalls.get(index));
        }
        assertEquals(core, (int) starter.typedCalls.get(starter.typedCalls.size() - 1));
        assertEquals(0, starter.untypedCalls);
    }

    @Test
    public void promotionFailureIsStructuredAndRollsBackReference() throws Exception {
        RecordingStarter starter = new RecordingStarter();
        ForegroundServiceTypes types = new ForegroundServiceTypes(34, starter);
        int core = ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE;

        types.startCore();
        starter.nextFailure = new SecurityException("camera permission missing");
        try {
            types.acquireCamera();
            fail("sensor promotion must fail closed");
        } catch (BridgeServer.BridgeError error) {
            assertEquals("E_PERMISSION", error.code);
            assertTrue(error.getMessage().contains("open AU Bridge"));
        }

        try (ForegroundServiceTypes.Lease ignored = types.acquireCamera()) {
            // A successful retry must hold exactly one reference.
        }
        assertEquals(Arrays.asList(
                core,
                core | ServiceInfo.FOREGROUND_SERVICE_TYPE_CAMERA,
                core), starter.typedCalls);
    }

    @Test
    public void demotionFailureRestoresReferenceAndAllowsCloseRetry() throws Exception {
        RecordingStarter starter = new RecordingStarter();
        ForegroundServiceTypes types = new ForegroundServiceTypes(34, starter);
        int core = ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE;
        int camera = core | ServiceInfo.FOREGROUND_SERVICE_TYPE_CAMERA;

        types.startCore();
        ForegroundServiceTypes.Lease lease = types.acquireCamera();
        starter.nextFailure = new SecurityException("transient foreground policy failure");
        try {
            lease.close();
            fail("failed demotion must keep the lease retryable");
        } catch (BridgeServer.BridgeError error) {
            assertEquals("E_PERMISSION", error.code);
            assertTrue(error.getMessage().contains("after the command"));
        }
        lease.close();
        lease.close();

        assertEquals(Arrays.asList(core, camera, core), starter.typedCalls);
    }

    @Test
    public void unexpectedPromotionFailureIsHelperErrorAndRollsBackReference() throws Exception {
        RecordingStarter starter = new RecordingStarter();
        ForegroundServiceTypes types = new ForegroundServiceTypes(34, starter);
        int core = ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE;
        int camera = core | ServiceInfo.FOREGROUND_SERVICE_TYPE_CAMERA;

        types.startCore();
        starter.nextFailure = new IllegalStateException("programming failure");
        try {
            types.acquireCamera();
            fail("unexpected transition failures must fail closed");
        } catch (BridgeServer.BridgeError error) {
            assertEquals("E_HELPER", error.code);
            assertTrue(error.getMessage().contains("inconsistent"));
        }
        try (ForegroundServiceTypes.Lease ignored = types.acquireCamera()) {
            // The failed acquire must not leave a hidden reference behind.
        }

        assertEquals(Arrays.asList(core, camera, core), starter.typedCalls);
    }

    private static void assertLegacyUntypedForegroundCalls(int sdkInt) throws Exception {
        RecordingStarter starter = new RecordingStarter();
        ForegroundServiceTypes types = new ForegroundServiceTypes(sdkInt, starter);

        types.startCore();
        try (ForegroundServiceTypes.Lease ignored = types.acquireCamera()) {
            // API 26-28 do not have the type-aware overload.
        }
        try (ForegroundServiceTypes.Lease ignored = types.acquireMicrophone()) {
            // API 26-28 do not have the type-aware overload.
        }

        assertEquals(5, starter.untypedCalls);
        assertTrue(starter.typedCalls.isEmpty());
    }

    private static final class RecordingStarter implements ForegroundServiceTypes.Starter {
        final List<Integer> typedCalls = new ArrayList<>();
        int untypedCalls;
        RuntimeException nextFailure;

        @Override
        public void startUntyped() {
            maybeFail();
            untypedCalls++;
        }

        @Override
        public void startTyped(int types) {
            maybeFail();
            typedCalls.add(types);
        }

        private void maybeFail() {
            if (nextFailure != null) {
                RuntimeException failure = nextFailure;
                nextFailure = null;
                throw failure;
            }
        }
    }
}
