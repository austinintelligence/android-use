package dev.codex.aubridge;

import android.Manifest;
import android.content.Context;
import android.content.pm.PackageManager;
import android.media.AudioFormat;
import android.media.AudioRecord;
import android.media.MediaRecorder;

import org.json.JSONObject;

import java.io.File;
import java.io.RandomAccessFile;

final class AudioCapture {
    private static final int SAMPLE_RATE = 16_000;
    private static final int CHANNELS = 1;

    interface Heartbeat {
        boolean fresh();
    }

    private AudioCapture() {
    }

    static JSONObject capture(Context context, int seconds, Heartbeat heartbeat) throws Exception {
        return captureInternal(context, seconds, heartbeat, true);
    }

    static JSONObject pcm(Context context, int seconds, Heartbeat heartbeat) throws Exception {
        return captureInternal(context, seconds, heartbeat, false);
    }

    private static JSONObject captureInternal(Context context, int seconds, Heartbeat heartbeat, boolean wav) throws Exception {
        if (context.checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
            throw new BridgeServer.BridgeError("E_PERMISSION", "Microphone permission is not granted");
        }
        seconds = Math.max(1, Math.min(seconds, 180));
        int channelMask = AudioFormat.CHANNEL_IN_MONO;
        int minimum = AudioRecord.getMinBufferSize(SAMPLE_RATE, channelMask, AudioFormat.ENCODING_PCM_16BIT);
        if (minimum <= 0) {
            throw new BridgeServer.BridgeError("E_CAPABILITY", "PCM16 microphone capture is not supported");
        }
        AudioRecord record = new AudioRecord.Builder()
                .setAudioSource(MediaRecorder.AudioSource.MIC)
                .setAudioFormat(new AudioFormat.Builder()
                        .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                        .setSampleRate(SAMPLE_RATE)
                        .setChannelMask(channelMask)
                        .build())
                .setBufferSizeInBytes(minimum * 2)
                .build();
        File output = new File(context.getFilesDir(), "media/mic-" + System.currentTimeMillis() + (wav ? ".wav" : ".pcm"));
        output.getParentFile().mkdirs();
        byte[] buffer = new byte[minimum];
        long dataBytes = 0L;
        long deadline = System.currentTimeMillis() + seconds * 1_000L;
        boolean completed = false;
        try (RandomAccessFile file = new RandomAccessFile(output, "rw")) {
            file.setLength(0L);
            if (wav) {
                file.write(new byte[44]);
            }
            record.startRecording();
            while (System.currentTimeMillis() < deadline) {
                if (!heartbeat.fresh()) {
                    throw new BridgeServer.BridgeError("E_HEARTBEAT", "host heartbeat expired during microphone capture");
                }
                int read = record.read(buffer, 0, buffer.length, AudioRecord.READ_BLOCKING);
                if (read > 0) {
                    file.write(buffer, 0, read);
                    dataBytes += read;
                }
            }
            if (wav) {
                file.seek(0L);
                writeWavHeader(file, dataBytes);
            }
            if (dataBytes == 0L) {
                throw new BridgeServer.BridgeError("E_MEDIA", "microphone capture contained no samples");
            }
            completed = true;
        } finally {
            if (record.getRecordingState() == AudioRecord.RECORDSTATE_RECORDING) record.stop();
            record.release();
            if (!completed) output.delete();
        }
        return new JSONObject()
                .put("file", "media/" + output.getName())
                .put("bytes", output.length())
                .put("duration_ms", seconds * 1_000L)
                .put("sample_rate", SAMPLE_RATE)
                .put("channels", CHANNELS)
                .put("sample_format", "pcm_s16le")
                .put("format", wav ? "wav-pcm16le" : "pcm_s16le");
    }

    private static void writeWavHeader(RandomAccessFile file, long dataBytes) throws Exception {
        int byteRate = SAMPLE_RATE * CHANNELS * 2;
        file.writeBytes("RIFF");
        writeLeInt(file, (int) (36 + dataBytes));
        file.writeBytes("WAVEfmt ");
        writeLeInt(file, 16);
        writeLeShort(file, (short) 1);
        writeLeShort(file, (short) CHANNELS);
        writeLeInt(file, SAMPLE_RATE);
        writeLeInt(file, byteRate);
        writeLeShort(file, (short) (CHANNELS * 2));
        writeLeShort(file, (short) 16);
        file.writeBytes("data");
        writeLeInt(file, (int) dataBytes);
    }

    private static void writeLeInt(RandomAccessFile file, int value) throws Exception {
        file.write(value & 0xff);
        file.write((value >>> 8) & 0xff);
        file.write((value >>> 16) & 0xff);
        file.write((value >>> 24) & 0xff);
    }

    private static void writeLeShort(RandomAccessFile file, short value) throws Exception {
        file.write(value & 0xff);
        file.write((value >>> 8) & 0xff);
    }
}
