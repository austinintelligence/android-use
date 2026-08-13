package dev.codex.aubridge;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;
import static org.junit.Assert.fail;

import org.junit.Test;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Set;

public final class PlanExecutorTest {
    @Test
    public void preflightRejectsDuplicateIdsAndUnknownOperations() throws Exception {
        assertPreflightError("E_ARGS", List.of(
                op("same", "back"),
                op("same", "stop")));
        assertPreflightError("E_ARGS", List.of(op("one", "install")));
    }

    @Test
    public void preflightRejectsOperationAndStringLimits() throws Exception {
        List<PlanExecutor.Operation> tooMany = new ArrayList<>();
        for (int index = 0; index <= PlanExecutor.MAX_OPERATIONS; index++) {
            tooMany.add(op("op" + index, "back"));
        }
        assertPreflightError("E_LIMIT", tooMany);
        assertPreflightError("E_LIMIT", List.of(
                op("wait", "wait.visible").selector(
                        "text~" + "x".repeat(PlanExecutor.MAX_SELECTOR_BYTES + 1))));

        PlanExecutor.Options options = options();
        options.deadlineMs = PlanExecutor.MAX_DEADLINE_MS + 1;
        try {
            PlanExecutor.preflight(List.of(op("stop", "stop")), options);
            fail("oversized deadline must fail preflight");
        } catch (BridgeServer.BridgeError error) {
            assertEquals("E_LIMIT", error.code);
        }
    }

    @Test
    public void preflightRejectsBackwardBranchesAndDependencyCycles() throws Exception {
        assertPreflightError("E_ARGS", List.of(
                op("first", "back"),
                op("branch", "if").selector("text~ready").branch("first", "")));
        assertPreflightError("E_ARGS", List.of(
                op("first", "back").dependsOn("second"),
                op("second", "back").dependsOn("first")));
    }

    @Test
    public void preflightFailureCannotReachMutationBackend() throws Exception {
        List<PlanExecutor.Operation> invalid = List.of(
                op("tap", "tap").target("text~go"),
                op("branch", "if").selector("text~ready").branch("tap", ""));
        PlanExecutor.CompiledPlan unchecked = new PlanExecutor.CompiledPlan(
                invalid, new HashMap<>(), options());
        FakeClock clock = new FakeClock();
        FakeUi ui = new FakeUi(clock);
        try {
            PlanExecutor.executeCompiled(unchecked, ui, clock, () -> false);
            fail("invalid plan must fail before execution");
        } catch (BridgeServer.BridgeError error) {
            assertEquals("E_ARGS", error.code);
        }
        assertEquals(0, ui.mutations);
        assertEquals(0, ui.visibilityChecks);
    }

    @Test
    public void forwardIfExecutesOnlySelectedBranch() throws Exception {
        List<PlanExecutor.Operation> operations = List.of(
                op("choose", "if").selector("text~ready").branch("yes", "no"),
                op("no", "tap").target("text~no"),
                op("no-stop", "stop"),
                op("yes", "tap").target("text~yes"),
                op("yes-stop", "stop"));
        FakeClock clock = new FakeClock();
        FakeUi ui = new FakeUi(clock);
        ui.visible.add("text~ready");

        PlanExecutor.ExecutionResult result = execute(operations, options(), ui, clock, () -> false);

        assertTrue(result.complete);
        assertTrue(result.stopped);
        assertEquals(1, result.committedMutations);
        assertEquals(List.of("text~yes"), ui.tapped);
        assertEquals("skipped", result.receipts.get(1).status);
        assertEquals("skipped", result.receipts.get(2).status);
        assertEquals("committed", result.receipts.get(3).status);
        assertEquals("accepted", result.receipts.get(4).status);
    }

    @Test
    public void deadlineAndCancellationAreBoundedAndExact() throws Exception {
        PlanExecutor.Operation wait = op("wait", "wait.visible")
                .selector("text~never")
                .timeout(100);
        FakeClock deadlineClock = new FakeClock();
        FakeUi deadlineUi = new FakeUi(deadlineClock);
        PlanExecutor.ExecutionResult timedOut = execute(
                List.of(wait), options(), deadlineUi, deadlineClock, () -> false);
        assertFalse(timedOut.complete);
        assertEquals("E_TIMEOUT", timedOut.error);
        assertEquals("failed", timedOut.receipts.get(0).status);
        assertTrue(deadlineUi.awaitCalls > 0);

        FakeClock cancelClock = new FakeClock();
        FakeUi cancelUi = new FakeUi(cancelClock);
        PlanExecutor.ExecutionResult cancelled = execute(
                List.of(op("back", "back")), options(), cancelUi, cancelClock, () -> true);
        assertFalse(cancelled.complete);
        assertEquals("E_CANCELLED", cancelled.error);
        assertTrue(cancelled.receipts.get(0).accepted);
        assertFalse(cancelled.receipts.get(0).committed);
        assertEquals(0, cancelUi.mutations);
    }

    @Test
    public void fuelExhaustionStopsPollingWithoutMutation() throws Exception {
        PlanExecutor.Options options = options();
        options.fuel = 2;
        FakeClock clock = new FakeClock();
        FakeUi ui = new FakeUi(clock);
        PlanExecutor.ExecutionResult result = execute(
                List.of(op("wait", "wait.visible").selector("text~never").timeout(1_000)),
                options,
                ui,
                clock,
                () -> false);
        assertFalse(result.complete);
        assertEquals("E_LIMIT", result.error);
        assertEquals(2, result.fuelUsed);
        assertEquals(0, ui.mutations);
    }

    @Test
    public void staleReferenceIsFailedAfterOneMutationAttemptAndNeverReplayed() throws Exception {
        FakeClock clock = new FakeClock();
        FakeUi ui = new FakeUi(clock);
        ui.tapFailure = new BridgeServer.BridgeError("E_STALE", "stale node handle");

        PlanExecutor.ExecutionResult result = execute(
                List.of(op("tap", "tap").target("123")),
                options(),
                ui,
                clock,
                () -> false);

        PlanExecutor.Receipt receipt = result.receipts.get(0);
        assertFalse(result.complete);
        assertEquals("E_STALE", result.error);
        assertEquals("failed", receipt.status);
        assertTrue(receipt.accepted);
        assertFalse(receipt.committed);
        assertFalse(receipt.observed);
        assertEquals(1, ui.tapAttempts);
        assertEquals(0, ui.mutations);
    }

    @Test
    public void failedPostconditionPreservesCommittedPartialReceipt() throws Exception {
        List<PlanExecutor.Operation> operations = List.of(
                op("tap", "tap").target("text~go"),
                op("assert", "assert.visible").selector("text~done").timeout(100),
                op("never", "back"));
        FakeClock clock = new FakeClock();
        FakeUi ui = new FakeUi(clock);

        PlanExecutor.ExecutionResult result = execute(
                operations, options(), ui, clock, () -> false);

        assertFalse(result.complete);
        assertEquals("E_ASSERT", result.error);
        assertEquals(1, result.committedMutations);
        assertEquals("committed", result.receipts.get(0).status);
        assertTrue(result.receipts.get(0).committed);
        assertFalse(result.receipts.get(0).observed);
        assertEquals("failed", result.receipts.get(1).status);
        assertEquals("skipped", result.receipts.get(2).status);
        assertEquals(1, ui.mutations);
    }

    @Test
    public void mutationObservationCanProveCommitWithoutReplayingAction() throws Exception {
        PlanExecutor.Operation tap = op("tap", "tap")
                .target("text~go")
                .observe("text~done", false, 500);
        FakeClock clock = new FakeClock();
        FakeUi ui = new FakeUi(clock);
        ui.visibleAfterTap = "text~done";

        PlanExecutor.ExecutionResult result = execute(
                List.of(tap), options(), ui, clock, () -> false);

        assertTrue(result.complete);
        assertEquals("observed", result.receipts.get(0).status);
        assertTrue(result.receipts.get(0).committed);
        assertTrue(result.receipts.get(0).observed);
        assertEquals(1, ui.tapAttempts);
    }

    @Test
    public void emptyTextIsAValidSingleMutationAndStateBudgetIsBounded() throws Exception {
        PlanExecutor.Options options = options();
        options.observationBudgetBytes = 0;
        FakeClock clock = new FakeClock();
        FakeUi ui = new FakeUi(clock);

        PlanExecutor.ExecutionResult result = execute(
                List.of(op("clear", "text").target("id=field").text("")),
                options,
                ui,
                clock,
                () -> false);

        assertTrue(result.complete);
        assertEquals(1, result.committedMutations);
        assertTrue(result.observationBudgetExhausted);
        assertEquals(null, result.receipts.get(0).state);
    }

    @Test
    public void scrollIsOneBoundedMutationWithValidatedDirection() throws Exception {
        FakeClock clock = new FakeClock();
        FakeUi ui = new FakeUi(clock);
        PlanExecutor.ExecutionResult result = execute(
                List.of(op("scroll", "scroll")
                        .target("scrollable=true#0")
                        .direction("forward")),
                options(), ui, clock, () -> false);

        assertTrue(result.complete);
        assertEquals(1, result.committedMutations);
        assertEquals(List.of("scrollable=true#0:forward"), ui.scrolled);

        assertPreflightError("E_ARGS", List.of(
                op("bad", "scroll").target("scrollable=true#0").direction("sideways")));
    }

    @Test
    public void errorBranchContinuesForwardWithCommittedFailureReceipt() throws Exception {
        PlanExecutor.Operation tap = op("tap", "tap")
                .target("text~go")
                .observe("text~never", false, 100)
                .onError("jump", "recover");
        List<PlanExecutor.Operation> operations = List.of(
                tap,
                op("skipped", "back"),
                op("recover", "back"));
        FakeClock clock = new FakeClock();
        FakeUi ui = new FakeUi(clock);

        PlanExecutor.ExecutionResult result = execute(
                operations, options(), ui, clock, () -> false);

        assertFalse(result.complete);
        assertEquals("E_ASSERT", result.error);
        assertTrue(result.receipts.get(0).committed);
        assertEquals("failed", result.receipts.get(0).status);
        assertEquals("skipped", result.receipts.get(1).status);
        assertEquals("committed", result.receipts.get(2).status);
        assertEquals(2, result.committedMutations);
        assertEquals(1, ui.tapAttempts);
    }

    private static PlanExecutor.ExecutionResult execute(
            List<PlanExecutor.Operation> operations,
            PlanExecutor.Options options,
            FakeUi ui,
            FakeClock clock,
            PlanExecutor.Cancellation cancellation) throws Exception {
        PlanExecutor.CompiledPlan plan = PlanExecutor.preflight(operations, options);
        return PlanExecutor.executeCompiled(plan, ui, clock, cancellation);
    }

    private static void assertPreflightError(
            String code, List<PlanExecutor.Operation> operations) throws Exception {
        try {
            PlanExecutor.preflight(operations, options());
            fail("preflight must reject invalid plan");
        } catch (BridgeServer.BridgeError error) {
            assertEquals(code, error.code);
        }
    }

    private static PlanExecutor.Operation op(String id, String kind) {
        return new PlanExecutor.Operation(id, kind);
    }

    private static PlanExecutor.Options options() {
        return new PlanExecutor.Options();
    }

    private static final class FakeClock implements PlanExecutor.Clock {
        long now;

        @Override
        public long nanoTime() {
            return now;
        }

        void advance(long milliseconds) {
            now += milliseconds * 1_000_000L;
        }
    }

    private static final class FakeUi implements PlanExecutor.Ui {
        final FakeClock clock;
        final Set<String> visible = new HashSet<>();
        final List<String> tapped = new ArrayList<>();
        final List<String> scrolled = new ArrayList<>();
        int generation = 1;
        int visibilityChecks;
        int awaitCalls;
        int tapAttempts;
        int mutations;
        String visibleAfterTap = "";
        BridgeServer.BridgeError tapFailure;

        FakeUi(FakeClock clock) {
            this.clock = clock;
        }

        @Override
        public boolean planVisible(String selector) {
            visibilityChecks++;
            return visible.contains(selector);
        }

        @Override
        public void planTap(String target) throws Exception {
            tapAttempts++;
            if (tapFailure != null) {
                throw tapFailure;
            }
            tapped.add(target);
            mutations++;
            generation++;
            if (!visibleAfterTap.isEmpty()) {
                visible.add(visibleAfterTap);
            }
        }

        @Override
        public void planText(String target, String text) {
            mutations++;
            generation++;
        }

        @Override
        public void planScroll(String target, String direction) {
            scrolled.add(target + ":" + direction);
            mutations++;
            generation++;
        }

        @Override
        public void planBack() {
            mutations++;
            generation++;
        }

        @Override
        public long planGeneration() {
            return generation;
        }

        @Override
        public void planAwaitChange(long observedGeneration, long timeoutMs) {
            awaitCalls++;
            clock.advance(timeoutMs);
        }

        @Override
        public PlanExecutor.State planState() {
            return new PlanExecutor.State(generation, 7, "h" + generation);
        }
    }
}
