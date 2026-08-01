package dev.codex.aubridge;

import android.Manifest;
import android.content.Context;
import android.content.pm.PackageManager;
import android.graphics.ImageFormat;
import android.hardware.camera2.CameraCaptureSession;
import android.hardware.camera2.CameraCharacteristics;
import android.hardware.camera2.CameraDevice;
import android.hardware.camera2.CameraManager;
import android.hardware.camera2.CaptureRequest;
import android.hardware.camera2.params.StreamConfigurationMap;
import android.media.Image;
import android.media.ImageReader;
import android.media.MediaRecorder;
import android.os.Handler;
import android.os.HandlerThread;
import android.util.Size;
import android.view.Surface;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.nio.ByteBuffer;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;

final class CameraCapture {
    interface Heartbeat {
        boolean fresh();
    }

    private CameraCapture() {
    }

    static JSONObject list(Context context) throws Exception {
        CameraManager manager = context.getSystemService(CameraManager.class);
        JSONArray cameras = new JSONArray();
        for (String id : manager.getCameraIdList()) {
            CameraCharacteristics characteristics = manager.getCameraCharacteristics(id);
            int facing = characteristics.get(CameraCharacteristics.LENS_FACING) == null ? -1 : characteristics.get(CameraCharacteristics.LENS_FACING);
            cameras.put(new JSONObject()
                    .put("id", id)
                    .put("facing", facing == CameraCharacteristics.LENS_FACING_FRONT ? "front" : facing == CameraCharacteristics.LENS_FACING_BACK ? "rear" : "external")
                    .put("orientation", characteristics.get(CameraCharacteristics.SENSOR_ORIENTATION)));
        }
        return new JSONObject().put("cameras", cameras);
    }

    static JSONObject snapshot(Context context, String requested, Heartbeat heartbeat) throws Exception {
        if (context.checkSelfPermission(Manifest.permission.CAMERA) != PackageManager.PERMISSION_GRANTED) {
            throw new BridgeServer.BridgeError("E_PERMISSION", "Camera permission is not granted");
        }
        if (!heartbeat.fresh()) {
            throw new BridgeServer.BridgeError("E_HEARTBEAT", "host heartbeat expired before camera start");
        }
        CameraManager manager = context.getSystemService(CameraManager.class);
        String cameraId = chooseCamera(manager, requested);
        Size size = chooseJpegSize(manager, cameraId);
        HandlerThread thread = new HandlerThread("au-camera");
        thread.start();
        Handler handler = new Handler(thread.getLooper());
        ImageReader reader = ImageReader.newInstance(size.getWidth(), size.getHeight(), ImageFormat.JPEG, 2);
        CountDownLatch latch = new CountDownLatch(1);
        final Exception[] failure = new Exception[1];
        final File output = new File(context.getFilesDir(), "media/camera-" + System.currentTimeMillis() + ".jpg");
        output.getParentFile().mkdirs();
        final CameraDevice[] device = new CameraDevice[1];
        final CameraCaptureSession[] session = new CameraCaptureSession[1];
        boolean completed = false;
        reader.setOnImageAvailableListener(source -> {
            try (Image image = source.acquireLatestImage(); FileOutputStream stream = new FileOutputStream(output)) {
                if (image == null || !heartbeat.fresh()) {
                    throw new IllegalStateException("camera heartbeat expired");
                }
                ByteBuffer buffer = image.getPlanes()[0].getBuffer();
                byte[] bytes = new byte[buffer.remaining()];
                buffer.get(bytes);
                stream.write(bytes);
                stream.getFD().sync();
            } catch (Exception error) {
                failure[0] = error;
            } finally {
                latch.countDown();
            }
        }, handler);
        try {
            manager.openCamera(cameraId, new CameraDevice.StateCallback() {
                @Override
                public void onOpened(CameraDevice opened) {
                    device[0] = opened;
                    try {
                        List<Surface> surfaces = new ArrayList<>();
                        surfaces.add(reader.getSurface());
                        opened.createCaptureSession(surfaces, new CameraCaptureSession.StateCallback() {
                            @Override
                            public void onConfigured(CameraCaptureSession configured) {
                                session[0] = configured;
                                try {
                                    CaptureRequest.Builder builder = opened.createCaptureRequest(CameraDevice.TEMPLATE_STILL_CAPTURE);
                                    builder.addTarget(reader.getSurface());
                                    CaptureRequest request = builder.build();
                                    configured.capture(request, new CameraCaptureSession.CaptureCallback() {}, handler);
                                } catch (Exception error) {
                                    failure[0] = error;
                                    latch.countDown();
                                }
                            }

                            @Override
                            public void onConfigureFailed(CameraCaptureSession ignored) {
                                failure[0] = new IllegalStateException("camera capture session failed");
                                latch.countDown();
                            }
                        }, handler);
                    } catch (Exception error) {
                        failure[0] = error;
                        latch.countDown();
                    }
                }

                @Override
                public void onDisconnected(CameraDevice opened) {
                    opened.close();
                    failure[0] = new IllegalStateException("camera disconnected");
                    latch.countDown();
                }

                @Override
                public void onError(CameraDevice opened, int error) {
                    opened.close();
                    failure[0] = new IllegalStateException("camera error " + error);
                    latch.countDown();
                }
            }, handler);
            if (!latch.await(8, TimeUnit.SECONDS)) {
                throw new BridgeServer.BridgeError("E_TIMEOUT", "camera snapshot timed out");
            }
            if (failure[0] != null) {
                throw failure[0];
            }
            if (!output.isFile() || output.length() == 0L) {
                throw new BridgeServer.BridgeError("E_MEDIA", "camera produced no JPEG");
            }
            completed = true;
            return new JSONObject().put("file", "media/" + output.getName()).put("bytes", output.length()).put("camera", cameraId).put("width", size.getWidth()).put("height", size.getHeight()).put("format", "jpeg");
        } finally {
            if (session[0] != null) session[0].close();
            if (device[0] != null) device[0].close();
            reader.close();
            thread.quitSafely();
            if (!completed) output.delete();
        }
    }

    static JSONObject record(Context context, String requested, int seconds, Heartbeat heartbeat) throws Exception {
        if (context.checkSelfPermission(Manifest.permission.CAMERA) != PackageManager.PERMISSION_GRANTED) {
            throw new BridgeServer.BridgeError("E_PERMISSION", "Camera permission is not granted");
        }
        if (!heartbeat.fresh()) {
            throw new BridgeServer.BridgeError("E_HEARTBEAT", "host heartbeat expired before camera start");
        }
        seconds = Math.max(1, Math.min(seconds, 180));
        CameraManager manager = context.getSystemService(CameraManager.class);
        String cameraId = chooseCamera(manager, requested);
        Size size = chooseVideoSize(manager, cameraId);
        File output = new File(context.getFilesDir(), "media/camera-" + System.currentTimeMillis() + ".mp4");
        output.getParentFile().mkdirs();
        HandlerThread thread = new HandlerThread("au-camera-record");
        thread.start();
        Handler handler = new Handler(thread.getLooper());
        MediaRecorder recorder = new MediaRecorder();
        CameraDevice[] device = new CameraDevice[1];
        CameraCaptureSession[] session = new CameraCaptureSession[1];
        Exception[] failure = new Exception[1];
        CountDownLatch ready = new CountDownLatch(1);
        boolean recording = false;
        boolean completed = false;
        try {
            recorder.setVideoSource(MediaRecorder.VideoSource.SURFACE);
            recorder.setOutputFormat(MediaRecorder.OutputFormat.MPEG_4);
            recorder.setVideoEncoder(MediaRecorder.VideoEncoder.H264);
            recorder.setVideoSize(size.getWidth(), size.getHeight());
            recorder.setVideoFrameRate(30);
            recorder.setVideoEncodingBitRate(Math.max(2_000_000, size.getWidth() * size.getHeight() * 5));
            recorder.setOutputFile(output.getAbsolutePath());
            recorder.prepare();
            Surface recorderSurface = recorder.getSurface();
            manager.openCamera(cameraId, new CameraDevice.StateCallback() {
                @Override
                public void onOpened(CameraDevice opened) {
                    device[0] = opened;
                    try {
                        List<Surface> surfaces = new ArrayList<>();
                        surfaces.add(recorderSurface);
                        opened.createCaptureSession(surfaces, new CameraCaptureSession.StateCallback() {
                            @Override
                            public void onConfigured(CameraCaptureSession configuredSession) {
                                session[0] = configuredSession;
                                try {
                                    CaptureRequest.Builder builder = opened.createCaptureRequest(CameraDevice.TEMPLATE_RECORD);
                                    builder.addTarget(recorderSurface);
                                    configuredSession.setRepeatingRequest(builder.build(), null, handler);
                                } catch (Exception error) {
                                    failure[0] = error;
                                } finally {
                                    ready.countDown();
                                }
                            }

                            @Override
                            public void onConfigureFailed(CameraCaptureSession ignored) {
                                failure[0] = new IllegalStateException("camera recording session failed");
                                ready.countDown();
                            }
                        }, handler);
                    } catch (Exception error) {
                        failure[0] = error;
                        ready.countDown();
                    }
                }

                @Override
                public void onDisconnected(CameraDevice opened) {
                    opened.close();
                    failure[0] = new IllegalStateException("camera disconnected");
                    ready.countDown();
                }

                @Override
                public void onError(CameraDevice opened, int error) {
                    opened.close();
                    failure[0] = new IllegalStateException("camera error " + error);
                    ready.countDown();
                }
            }, handler);
            if (!ready.await(8, TimeUnit.SECONDS)) {
                throw new BridgeServer.BridgeError("E_TIMEOUT", "camera recording setup timed out");
            }
            if (failure[0] != null) {
                throw failure[0];
            }
            recorder.start();
            recording = true;
            long deadline = System.currentTimeMillis() + seconds * 1_000L;
            while (System.currentTimeMillis() < deadline) {
                if (!heartbeat.fresh()) {
                    throw new BridgeServer.BridgeError("E_HEARTBEAT", "host heartbeat expired during camera recording");
                }
                Thread.sleep(100L);
            }
            recorder.stop();
            recording = false;
            if (!output.isFile() || output.length() == 0L) {
                throw new BridgeServer.BridgeError("E_MEDIA", "camera recording produced no MP4");
            }
            completed = true;
            return new JSONObject()
                    .put("file", "media/" + output.getName())
                    .put("bytes", output.length())
                    .put("camera", cameraId)
                    .put("width", size.getWidth())
                    .put("height", size.getHeight())
                    .put("duration_ms", seconds * 1_000L)
                    .put("format", "mp4-h264");
        } finally {
            if (recording) {
                try {
                    recorder.stop();
                } catch (Exception ignored) {
                }
            }
            if (session[0] != null) session[0].close();
            if (device[0] != null) device[0].close();
            recorder.release();
            thread.quitSafely();
            if (!completed) output.delete();
        }
    }

    /** A finite multipart MJPEG stream. The host may emit it only with --binary. */
    static JSONObject mjpeg(Context context, String requested, int seconds, Heartbeat heartbeat) throws Exception {
        seconds = Math.max(1, Math.min(seconds, 30));
        File output = new File(context.getFilesDir(), "media/camera-" + System.currentTimeMillis() + ".mjpeg");
        output.getParentFile().mkdirs();
        long deadline = System.currentTimeMillis() + seconds * 1_000L;
        int frames = 0;
        int width = 0;
        int height = 0;
        String camera = "";
        boolean completed = false;
        try {
            try (FileOutputStream stream = new FileOutputStream(output)) {
            while (System.currentTimeMillis() < deadline) {
                if (!heartbeat.fresh()) {
                    throw new BridgeServer.BridgeError("E_HEARTBEAT", "host heartbeat expired during camera stream");
                }
                JSONObject jpeg = snapshot(context, requested, heartbeat);
                String relative = jpeg.getString("file");
                File frame = new File(context.getFilesDir(), relative);
                byte[] header = ("--au\r\nContent-Type: image/jpeg\r\nContent-Length: " + frame.length() + "\r\n\r\n")
                        .getBytes(java.nio.charset.StandardCharsets.US_ASCII);
                stream.write(header);
                try (FileInputStream input = new FileInputStream(frame)) {
                    byte[] buffer = new byte[16 * 1024];
                    int count;
                    while ((count = input.read(buffer)) >= 0) {
                        stream.write(buffer, 0, count);
                    }
                }
                stream.write("\r\n".getBytes(java.nio.charset.StandardCharsets.US_ASCII));
                frame.delete();
                frames++;
                width = jpeg.optInt("width", width);
                height = jpeg.optInt("height", height);
                camera = jpeg.optString("camera", camera);
                Thread.sleep(120L);
            }
            stream.write("--au--\r\n".getBytes(java.nio.charset.StandardCharsets.US_ASCII));
            stream.getFD().sync();
            }
            if (frames == 0 || output.length() == 0L) {
                throw new BridgeServer.BridgeError("E_MEDIA", "camera MJPEG stream contained no frames");
            }
            completed = true;
            return new JSONObject()
                .put("file", "media/" + output.getName())
                .put("bytes", output.length())
                .put("frames", frames)
                .put("camera", camera)
                .put("width", width)
                .put("height", height)
                .put("boundary", "au")
                .put("format", "multipart-x-mixed-replace; boundary=au");
        } finally {
            if (!completed) output.delete();
        }
    }

    private static String chooseCamera(CameraManager manager, String requested) throws Exception {
        for (String id : manager.getCameraIdList()) {
            CameraCharacteristics characteristics = manager.getCameraCharacteristics(id);
            Integer facing = characteristics.get(CameraCharacteristics.LENS_FACING);
            if (requested.equals(id)
                    || (requested.equals("front") && facing != null && facing == CameraCharacteristics.LENS_FACING_FRONT)
                    || ((requested.isEmpty() || requested.equals("rear")) && facing != null && facing == CameraCharacteristics.LENS_FACING_BACK)) {
                return id;
            }
        }
        String[] cameras = manager.getCameraIdList();
        if (cameras.length == 0) {
            throw new BridgeServer.BridgeError("E_CAPABILITY", "No camera is available");
        }
        return cameras[0];
    }

    private static Size chooseJpegSize(CameraManager manager, String cameraId) throws Exception {
        StreamConfigurationMap map = manager.getCameraCharacteristics(cameraId)
                .get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP);
        if (map == null || map.getOutputSizes(ImageFormat.JPEG) == null) {
            throw new BridgeServer.BridgeError("E_CAPABILITY", "camera has no JPEG output size");
        }
        return chooseSize(map.getOutputSizes(ImageFormat.JPEG));
    }

    private static Size chooseVideoSize(CameraManager manager, String cameraId) throws Exception {
        StreamConfigurationMap map = manager.getCameraCharacteristics(cameraId)
                .get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP);
        if (map == null || map.getOutputSizes(MediaRecorder.class) == null) {
            throw new BridgeServer.BridgeError("E_CAPABILITY", "camera has no MediaRecorder output size");
        }
        return chooseSize(map.getOutputSizes(MediaRecorder.class));
    }

    private static Size chooseSize(Size[] sizes) {
        Size selected = sizes[0];
        long selectedArea = 0L;
        for (Size size : sizes) {
            long area = (long) size.getWidth() * size.getHeight();
            if (size.getWidth() <= 1280 && size.getHeight() <= 1280 && area > selectedArea) {
                selected = size;
                selectedArea = area;
            }
        }
        return selected;
    }
}
