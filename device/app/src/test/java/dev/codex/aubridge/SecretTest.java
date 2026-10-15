package dev.codex.aubridge;

import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import android.text.InputType;

import org.junit.Test;

public class SecretTest {
    private static final int TEXT = InputType.TYPE_CLASS_TEXT;

    @Test
    public void textWebAndNumberPasswordsAreDetected() {
        assertTrue(Secret.password(TEXT | InputType.TYPE_TEXT_VARIATION_PASSWORD));
        assertTrue(
                Secret.password(
                        TEXT
                                | InputType.TYPE_TEXT_VARIATION_WEB_PASSWORD
                                | InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS));
        assertTrue(
                Secret.password(
                        InputType.TYPE_CLASS_NUMBER | InputType.TYPE_NUMBER_VARIATION_PASSWORD));
    }

    @Test
    public void ordinaryAndVisiblePasswordFieldsAreNotRedacted() {
        assertFalse(Secret.password(0));
        assertFalse(Secret.password(TEXT));
        assertFalse(Secret.password(TEXT | InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD));
        assertFalse(Secret.password(TEXT | InputType.TYPE_TEXT_VARIATION_EMAIL_ADDRESS));
        assertFalse(Secret.password(InputType.TYPE_CLASS_NUMBER));
        assertFalse(Secret.password(InputType.TYPE_CLASS_PHONE));
    }
}
