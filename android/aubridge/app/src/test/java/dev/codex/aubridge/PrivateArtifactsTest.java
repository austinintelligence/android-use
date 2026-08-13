package dev.codex.aubridge;

import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import org.junit.Test;

public final class PrivateArtifactsTest {
    @Test
    public void acceptsOnlyOneBoundedMediaBasename() {
        assertTrue(PrivateArtifacts.isValidRelativePath("media/camera-123.jpg"));
        assertTrue(PrivateArtifacts.isValidRelativePath("media/mic_1.wav"));
        assertFalse(PrivateArtifacts.isValidRelativePath("bridge_token"));
        assertFalse(PrivateArtifacts.isValidRelativePath("media/../bridge_token"));
        assertFalse(PrivateArtifacts.isValidRelativePath("media/sub/file.jpg"));
        assertFalse(PrivateArtifacts.isValidRelativePath("/media/file.jpg"));
        assertFalse(PrivateArtifacts.isValidRelativePath("media/" + "x".repeat(129)));
    }
}
