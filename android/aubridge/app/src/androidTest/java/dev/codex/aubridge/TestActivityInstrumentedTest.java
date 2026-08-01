package dev.codex.aubridge;

import static org.junit.Assert.assertNotNull;

import androidx.test.core.app.ActivityScenario;
import androidx.test.ext.junit.runners.AndroidJUnit4;

import org.junit.Test;
import org.junit.runner.RunWith;

/** Keeps the intentionally harmless validation surface launchable on real devices. */
@RunWith(AndroidJUnit4.class)
public final class TestActivityInstrumentedTest {
    @Test
    public void deterministicActivityLaunches() {
        try (ActivityScenario<TestActivity> scenario = ActivityScenario.launch(TestActivity.class)) {
            scenario.onActivity(activity -> assertNotNull(activity.getWindow().getDecorView()));
        }
    }
}
