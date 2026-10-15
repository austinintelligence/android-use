package dev.codex.aubridge;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.fail;

import android.view.accessibility.AccessibilityEvent;

import org.json.JSONArray;
import org.json.JSONObject;
import org.junit.Test;

import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;

public final class WireTest {
    static JSONObject golden() throws Exception {
        for (String candidate :
                new String[] {
                    "protocol-golden.json", "../protocol-golden.json", "../../protocol-golden.json"
                }) {
            Path path = Paths.get(candidate);
            if (Files.isRegularFile(path))
                return new JSONObject(new String(Files.readAllBytes(path), StandardCharsets.UTF_8));
        }
        throw new java.io.FileNotFoundException("protocol-golden.json");
    }

    @Test
    public void framingAndPlanMatchRustGolden() throws Exception {
        JSONArray expected = golden().getJSONArray("frame");
        ByteArrayOutputStream bytes = new ByteArrayOutputStream();
        Bridge.write(new DataOutputStream(bytes), expected);
        JSONArray decoded =
                Bridge.read(new DataInputStream(new ByteArrayInputStream(bytes.toByteArray())));
        assertEquals(expected.toString(), decoded.toString());
        Ui.Scene scene = new Ui.Scene(44, "", new ArrayList<>(), 0, false);
        Vm.P p = Vm.P.parse(decoded.getJSONArray(6).getJSONArray(2), scene, null, null);
        assertEquals("wait", p.name);
        p.close();
    }

    @Test
    public void frameBoundsAreCheckedBeforeAllocation() throws Exception {
        byte[] bad = {0x00, 0x10, 0x00, 0x01};
        try {
            Bridge.read(new DataInputStream(new ByteArrayInputStream(bad)));
            fail();
        } catch (java.io.IOException expected) {
            assertEquals("frame", expected.getMessage());
        }
    }

    @Test
    public void branchesAreNotAnOperation() throws Exception {
        Ui.Scene scene = new Ui.Scene(1, "", new ArrayList<>(), 0, false);
        try {
            Vm.P.parse(new JSONArray("[\"branch\",1]"), scene, null, null);
            fail();
        } catch (Vm.Unsupported expected) {
        }
    }

    @Test
    public void targetRelevantAccessibilityChangesInvalidate() {
        assertEquals(
                true,
                Ui.shouldInvalidate(
                        AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED,
                        AccessibilityEvent.CONTENT_CHANGE_TYPE_TEXT));
        assertEquals(
                true,
                Ui.shouldInvalidate(
                        AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED,
                        AccessibilityEvent.CONTENT_CHANGE_TYPE_CONTENT_DESCRIPTION));
        assertEquals(
                true,
                Ui.shouldInvalidate(
                        AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED,
                        AccessibilityEvent.CONTENT_CHANGE_TYPE_STATE_DESCRIPTION));
        assertEquals(
                true,
                Ui.shouldInvalidate(
                        AccessibilityEvent.TYPE_VIEW_CLICKED,
                        AccessibilityEvent.CONTENT_CHANGE_TYPE_UNDEFINED));
        assertEquals(
                false,
                Ui.shouldInvalidate(
                        AccessibilityEvent.TYPE_ANNOUNCEMENT,
                        AccessibilityEvent.CONTENT_CHANGE_TYPE_UNDEFINED));
        assertEquals(
                false,
                Ui.shouldInvalidate(
                        AccessibilityEvent.TYPE_NOTIFICATION_STATE_CHANGED,
                        AccessibilityEvent.CONTENT_CHANGE_TYPE_UNDEFINED));
    }

    @Test
    public void mediaAndNotificationOpsStayBounded() throws Exception {
        Ui.Scene scene = new Ui.Scene(1, "", new ArrayList<>(), 0, false);
        Vm.P camera = Vm.P.parse(new JSONArray("[\"camera\",\"rear\",640,480]"), scene, null, null);
        assertEquals("camera", camera.name);
        camera.close();
        Vm.P mic = Vm.P.parse(new JSONArray("[\"microphone\",3]"), scene, null, null);
        assertEquals(3, mic.arg);
        mic.close();
        Vm.P record = Vm.P.parse(new JSONArray("[\"screen_record\",2]"), scene, null, null);
        assertEquals(2, record.arg);
        record.close();
        Vm.P notification =
                Vm.P.parse(new JSONArray("[\"notification_action\",\"key\"]"), scene, null, null);
        assertEquals("notification_action", notification.name);
        notification.close();
    }

    private static Ui.Scene scene(boolean truncated, String... labels) {
        ArrayList<Ui.N> nodes = new ArrayList<>();
        for (String label : labels)
            nodes.add(new Ui.N(label, 't', 0, null, 1, 0, label, "", false, true, 0, 0, 1, 1));
        return new Ui.Scene(1, "", nodes, nodes.size(), truncated);
    }

    @Test
    public void emptyNeedlesMatchNothingAndMissingFailsClosedWhenTruncated() throws Exception {
        assertEquals(false, scene(false, "Inbox").label("[]"));
        assertEquals(false, scene(false, "Inbox").named("\u2026"));
        try {
            Vm.Pred.parse(new JSONArray("[\"missing\",[\"label\",\"{}\"]]"), null);
            fail();
        } catch (Bridge.Limit expected) {
        }
        Vm.Pred gone = Vm.Pred.parse(new JSONArray("[\"missing\",[\"label\",\"Buy\"]]"), null);
        assertEquals(true, gone.test(scene(false, "Inbox")));
        assertEquals(false, gone.test(scene(true, "Inbox")));
    }
}
