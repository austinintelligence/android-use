package dev.codex.aubridge;

import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import android.text.InputType;

import org.junit.Test;

public class SecretTest {
    @Test
    public void everyPasswordVariationIsDetected() {
        assertTrue(Secret.password(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_PASSWORD));
        assertTrue(
                Secret.password(
                        InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD));
        assertTrue(
                Secret.password(
                        InputType.TYPE_CLASS_TEXT
                                | InputType.TYPE_TEXT_VARIATION_WEB_PASSWORD
                                | InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS));
        assertTrue(
                Secret.password(
                        InputType.TYPE_CLASS_NUMBER | InputType.TYPE_NUMBER_VARIATION_PASSWORD));
    }

    @Test
    public void ordinaryFieldsAreNotSecrets() {
        assertFalse(Secret.password(0));
        assertFalse(Secret.password(InputType.TYPE_CLASS_TEXT));
        assertFalse(
                Secret.password(
                        InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_EMAIL_ADDRESS));
        assertFalse(Secret.password(InputType.TYPE_CLASS_NUMBER));
        assertFalse(Secret.password(InputType.TYPE_CLASS_PHONE));
    }
}
