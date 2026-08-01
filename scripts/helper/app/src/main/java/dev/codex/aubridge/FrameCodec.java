package dev.codex.aubridge;

import java.io.EOFException;
import java.io.InputStream;
import java.io.OutputStream;

/** Length-prefixed little-endian helper frames with a hard allocation bound. */
final class FrameCodec {
    static final int MAX_FRAME = 1024 * 1024;

    private FrameCodec() {
    }

    static byte[] readFrame(InputStream input) throws Exception {
        byte[] header = readFully(input, 4);
        int length = (header[0] & 0xff)
                | ((header[1] & 0xff) << 8)
                | ((header[2] & 0xff) << 16)
                | ((header[3] & 0xff) << 24);
        if (length <= 0 || length > MAX_FRAME) {
            throw new FrameError("invalid frame size");
        }
        return readFully(input, length);
    }

    static void writeFrame(OutputStream output, byte[] body) throws Exception {
        if (body.length > MAX_FRAME) {
            throw new FrameError("response exceeds frame limit");
        }
        output.write(body.length & 0xff);
        output.write((body.length >>> 8) & 0xff);
        output.write((body.length >>> 16) & 0xff);
        output.write((body.length >>> 24) & 0xff);
        output.write(body);
        output.flush();
    }

    private static byte[] readFully(InputStream input, int length) throws Exception {
        byte[] body = new byte[length];
        int offset = 0;
        while (offset < length) {
            int count = input.read(body, offset, length - offset);
            if (count < 0) {
                throw new EOFException();
            }
            if (count == 0) {
                continue;
            }
            offset += count;
        }
        return body;
    }

    static final class FrameError extends Exception {
        FrameError(String message) {
            super(message);
        }
    }
}
