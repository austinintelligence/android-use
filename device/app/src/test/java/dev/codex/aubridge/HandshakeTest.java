package dev.codex.aubridge;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;
import static org.junit.Assert.fail;

import org.json.JSONArray;
import org.json.JSONObject;
import org.junit.Test;

import java.io.ByteArrayInputStream;
import java.io.Closeable;
import java.io.DataInputStream;
import java.io.IOException;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Base64;
import java.util.List;
import java.util.concurrent.atomic.AtomicInteger;

public final class HandshakeTest {
    private static String hex(byte[] bytes) {
        StringBuilder out = new StringBuilder();
        for (byte b : bytes) out.append(String.format("%02x", b));
        return out.toString();
    }

    private static final class Conn implements Closeable {
        final boolean bad;
        boolean closed;

        Conn(boolean bad) {
            this.bad = bad;
        }

        @Override
        public void close() {
            closed = true;
        }
    }

    @Test
    public void responderMatchesTheHmacOfChallengeAndContext() {
        byte[] key = new byte[32];
        java.util.Arrays.fill(key, (byte) 'k');
        byte[] challenge = new byte[16];
        for (int i = 0; i < 16; i++) challenge[i] = (byte) i;
        assertEquals(
                "458e603d51c1698e01e8867d2337e43666b4e266a6abe94fc4e275274ba29c13",
                hex(HelperKey.respond(key, challenge)));
    }

    @Test
    public void responderMatchesTheGoldenHandshake() throws Exception {
        JSONObject golden = WireTest.golden().getJSONObject("handshake");
        byte[] key = Base64.getDecoder().decode(golden.getString("key"));
        byte[] challenge =
                Base64.getUrlDecoder().decode(golden.getJSONArray("challenge").getString(2));
        String proof = golden.getJSONArray("reply").getString(4);
        assertEquals(
                proof, Base64.getUrlEncoder().encodeToString(HelperKey.respond(key, challenge)));
    }

    @Test
    public void commandResponderMatchesTheGoldenAndIsBoundToTheToken() throws Exception {
        JSONObject golden = WireTest.golden().getJSONObject("handshake");
        byte[] key = Base64.getDecoder().decode(golden.getString("key"));
        JSONArray hello = golden.getJSONArray("command");
        byte[] challenge = Base64.getUrlDecoder().decode(hello.getString(4));
        String proof = golden.getJSONArray("command_reply").getString(2);
        assertEquals(
                proof,
                Base64.getUrlEncoder()
                        .encodeToString(
                                HelperKey.respondCommand(key, challenge, hello.getString(2))));
        assertFalse(
                java.util.Arrays.equals(
                        Base64.getUrlDecoder().decode(proof),
                        HelperKey.respondCommand(key, challenge, "other")));
        assertFalse(
                java.util.Arrays.equals(
                        HelperKey.respond(key, challenge),
                        HelperKey.respondCommand(key, challenge, "")));
    }

    @Test
    public void aDifferentKeyOrChallengeGivesADifferentAnswer() {
        byte[] challenge = new byte[16];
        byte[] other = new byte[16];
        other[0] = 1;
        byte[] key = HelperKey.key();
        assertFalse(
                java.util.Arrays.equals(
                        HelperKey.respond(key, challenge), HelperKey.respond(key, other)));
        assertFalse(
                java.util.Arrays.equals(
                        HelperKey.respond(key, challenge),
                        HelperKey.respond(new byte[32], challenge)));
    }

    @Test
    public void onlyTheShellUidMayReadTheKey() {
        assertTrue(HelperKey.mayRead(2000));
        for (int uid : new int[] {0, 1000, 1999, 2001, 10123, -1})
            assertFalse(HelperKey.mayRead(uid));
    }

    @Test
    public void theKeyIsStableInAProcessAndCopiedOut() {
        byte[] first = HelperKey.key();
        assertEquals(32, first.length);
        first[0] ^= 1;
        assertFalse(first[0] == HelperKey.key()[0]);
    }

    @Test
    public void aFramingErrorClosesOnlyThatConnection() throws Exception {
        ArrayDeque<Conn> queue = new ArrayDeque<>();
        List<Conn> all = new ArrayList<>();
        for (boolean bad : new boolean[] {false, true, false}) {
            Conn c = new Conn(bad);
            queue.add(c);
            all.add(c);
        }
        AtomicInteger handled = new AtomicInteger();
        Serve.loop(
                () -> {
                    Conn c = queue.poll();
                    if (c == null) throw new IOException("done");
                    return c;
                },
                c -> {
                    handled.incrementAndGet();
                    if (c.bad) {
                        // An oversized length prefix is the framing error Bridge.read rejects.
                        Bridge.read(
                                new DataInputStream(
                                        new ByteArrayInputStream(new byte[] {0x7f, 0, 0, 0})));
                        fail("the frame must be rejected");
                    }
                },
                () -> !queue.isEmpty());
        assertEquals(3, handled.get());
        assertFalse(all.get(0).closed);
        assertTrue(all.get(1).closed);
        assertFalse(all.get(2).closed);
    }

    @Test
    public void anAcceptErrorDoesNotEndTheLoop() {
        AtomicInteger calls = new AtomicInteger();
        AtomicInteger served = new AtomicInteger();
        Serve.loop(
                () -> {
                    int n = calls.incrementAndGet();
                    if (n == 1) throw new IOException("transient");
                    return new Conn(false);
                },
                c -> served.incrementAndGet(),
                () -> calls.get() < 3);
        assertEquals(2, served.get());
    }

    @Test
    public void bindRetriesThenSucceeds() throws Exception {
        AtomicInteger tries = new AtomicInteger();
        String got =
                Serve.bind(
                        () -> {
                            if (tries.incrementAndGet() < 3) throw new IOException("in use");
                            return "bound";
                        },
                        5,
                        1);
        assertEquals("bound", got);
        assertEquals(3, tries.get());
    }

    @Test
    public void bindGivesUpAndSurfacesTheFailure() {
        AtomicInteger tries = new AtomicInteger();
        try {
            Serve.bind(
                    () -> {
                        tries.incrementAndGet();
                        throw new IOException("in use");
                    },
                    3,
                    1);
            fail("expected a failure");
        } catch (IOException e) {
            assertEquals("in use", e.getMessage());
        }
        assertEquals(3, tries.get());
    }
}
