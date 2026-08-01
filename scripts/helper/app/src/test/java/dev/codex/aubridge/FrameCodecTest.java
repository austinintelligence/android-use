package dev.codex.aubridge;

import static org.junit.Assert.assertArrayEquals;
import static org.junit.Assert.assertEquals;
import static org.junit.Assert.fail;

import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.InputStream;

import org.junit.Test;

public final class FrameCodecTest {
    @Test
    public void roundTripUsesLittleEndianLengthPrefix() throws Exception {
        byte[] body = "hello".getBytes(java.nio.charset.StandardCharsets.UTF_8);
        ByteArrayOutputStream output = new ByteArrayOutputStream();
        FrameCodec.writeFrame(output, body);
        assertArrayEquals(new byte[]{5, 0, 0, 0, 'h', 'e', 'l', 'l', 'o'}, output.toByteArray());
        assertArrayEquals(body, FrameCodec.readFrame(new ByteArrayInputStream(output.toByteArray())));
    }

    @Test
    public void readHandlesPartialInputReads() throws Exception {
        byte[] encoded = new byte[]{3, 0, 0, 0, 1, 2, 3};
        InputStream oneByteAtATime = new ByteArrayInputStream(encoded) {
            @Override
            public int read(byte[] target, int offset, int length) {
                return super.read(target, offset, Math.min(1, length));
            }
        };
        assertArrayEquals(new byte[]{1, 2, 3}, FrameCodec.readFrame(oneByteAtATime));
    }

    @Test
    public void rejectsZeroAndOversizedFramesBeforeAllocation() throws Exception {
        try {
            FrameCodec.readFrame(new ByteArrayInputStream(new byte[]{0, 0, 0, 0}));
            fail("zero-length frame must be rejected");
        } catch (FrameCodec.FrameError expected) {
            assertEquals("invalid frame size", expected.getMessage());
        }
        try {
            FrameCodec.writeFrame(new ByteArrayOutputStream(), new byte[FrameCodec.MAX_FRAME + 1]);
            fail("oversized frame must be rejected");
        } catch (FrameCodec.FrameError expected) {
            assertEquals("response exceeds frame limit", expected.getMessage());
        }
    }
}
