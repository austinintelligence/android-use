package dev.codex.aubridge;

import org.json.JSONArray;
import org.json.JSONObject;

import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.HashSet;
import java.util.Iterator;
import java.util.List;
import java.util.Map;
import java.util.Set;

/**
 * A bounded, forward-only semantic plan interpreter.
 *
 * <p>The bridge authenticates and sequences the containing frame before this
 * class is entered. This class performs complete structural preflight before
 * calling {@link Ui}, never retries a mutation, and has no package-manager,
 * download, submission, shell, recursion, or thread primitive.</p>
 */
final class PlanExecutor {
    static final int MAX_OPERATIONS = 128;
    static final int MAX_FUEL = 512;
    static final int DEFAULT_DEADLINE_MS = 30_000;
    static final int MAX_DEADLINE_MS = 60_000;
    static final int MAX_SELECTOR_BYTES = 1_024;
    static final int MAX_TEXT_BYTES = 8_192;
    static final int MAX_ID_BYTES = 64;
    static final int MAX_TOTAL_STRING_BYTES = 128 * 1_024;
    static final int DEFAULT_OBSERVATION_BUDGET_BYTES = 16 * 1_024;
    static final int MAX_OBSERVATION_BUDGET_BYTES = 32 * 1_024;
    private static final int MAX_ERROR_MESSAGE_BYTES = 512;
    private static final long FALLBACK_POLL_MS = 100L;

    private PlanExecutor() {
    }

    interface Ui {
        boolean planVisible(String selector) throws Exception;

        void planTap(String target) throws Exception;

        void planText(String target, String text) throws Exception;

        void planScroll(String target, String direction) throws Exception;

        void planBack() throws Exception;

        long planGeneration();

        void planAwaitChange(long observedGeneration, long timeoutMs) throws Exception;

        State planState() throws Exception;
    }

    interface Clock {
        long nanoTime();
    }

    interface Cancellation {
        boolean cancelled();
    }

    static final class State {
        final long generation;
        final int window;
        final String hash;

        State(long generation, int window, String hash) {
            this.generation = generation;
            this.window = window;
            this.hash = hash == null ? "" : capUtf8(hash, 64);
        }
    }

    static final class Options {
        int deadlineMs = DEFAULT_DEADLINE_MS;
        int fuel = MAX_FUEL;
        int observationBudgetBytes = DEFAULT_OBSERVATION_BUDGET_BYTES;
        boolean diagnostic;
    }

    static final class Operation {
        final String id;
        final String kind;
        String target = "";
        String selector = "";
        String text = "";
        String direction = "forward";
        String thenId = "";
        String elseId = "";
        String onError = "stop";
        String errorTarget = "";
        String observe = "";
        boolean observeNotVisible;
        int timeoutMs = 3_000;
        int observeTimeoutMs = 3_000;
        final List<String> dependencies = new ArrayList<>();

        Operation(String id, String kind) {
            this.id = id;
            this.kind = kind;
        }

        Operation target(String value) {
            target = value;
            return this;
        }

        Operation selector(String value) {
            selector = value;
            return this;
        }

        Operation text(String value) {
            text = value;
            return this;
        }

        Operation direction(String value) {
            direction = value;
            return this;
        }

        Operation branch(String thenValue, String elseValue) {
            thenId = thenValue;
            elseId = elseValue;
            return this;
        }

        Operation dependsOn(String... ids) {
            Collections.addAll(dependencies, ids);
            return this;
        }

        Operation onError(String policy, String targetId) {
            onError = policy;
            errorTarget = targetId;
            return this;
        }

        Operation observe(String value, boolean notVisible, int timeout) {
            observe = value;
            observeNotVisible = notVisible;
            observeTimeoutMs = timeout;
            return this;
        }

        Operation timeout(int value) {
            timeoutMs = value;
            return this;
        }

        Operation copy() {
            Operation copy = new Operation(id, kind);
            copy.target = target;
            copy.selector = selector;
            copy.text = text;
            copy.direction = direction;
            copy.thenId = thenId;
            copy.elseId = elseId;
            copy.onError = onError;
            copy.errorTarget = errorTarget;
            copy.observe = observe;
            copy.observeNotVisible = observeNotVisible;
            copy.timeoutMs = timeoutMs;
            copy.observeTimeoutMs = observeTimeoutMs;
            copy.dependencies.addAll(dependencies);
            return copy;
        }
    }

    static final class CompiledPlan {
        final List<Operation> operations;
        final Map<String, Integer> indexes;
        final Options options;

        CompiledPlan(List<Operation> operations, Map<String, Integer> indexes, Options options) {
            this.operations = operations;
            this.indexes = indexes;
            this.options = options;
        }
    }

    static final class Receipt {
        final String id;
        final String operation;
        String status = "accepted";
        boolean accepted;
        boolean committed;
        boolean observed;
        long elapsedMs;
        State state;
        String error = "";
        String message = "";
        String reason = "";

        Receipt(Operation operation) {
            id = operation.id;
            this.operation = operation.kind;
        }

        boolean successful() {
            return "accepted".equals(status)
                    || "committed".equals(status)
                    || "observed".equals(status);
        }
    }

    static final class ExecutionResult {
        final List<Receipt> receipts;
        boolean complete = true;
        boolean stopped;
        boolean observationBudgetExhausted;
        int committedMutations;
        int failedOperations;
        int skippedOperations;
        int fuelUsed;
        long elapsedMs;
        State finalState;
        String error = "";
        String message = "";
        String failedId = "";
        int failedIndex = -1;

        ExecutionResult(int operationCount) {
            receipts = new ArrayList<>(Collections.nCopies(operationCount, null));
        }
    }

    static JSONObject execute(Ui ui, JSONObject args) throws Exception {
        CompiledPlan plan = parse(args);
        ExecutionResult result = executeCompiled(
                plan,
                ui,
                System::nanoTime,
                () -> Thread.currentThread().isInterrupted());
        return encode(result, plan.options.diagnostic);
    }

    static CompiledPlan parse(JSONObject args) throws Exception {
        // FrameCodec has already bounded the complete request before JSON
        // decoding. Re-serializing the JSONObject here both duplicates that
        // guard and makes JVM-only parser tests depend on Android's mocked
        // JSONObject.toString implementation.
        rejectUnknown(args, allowed(
                "operations", "deadline_ms", "fuel", "observation_budget_bytes", "diagnostic"),
                "plan");
        JSONArray encodedOperations = args.optJSONArray("operations");
        if (encodedOperations == null) {
            throw new BridgeServer.BridgeError("E_ARGS", "plan.run requires an operations array");
        }
        Options options = new Options();
        options.deadlineMs = integer(args, "deadline_ms", DEFAULT_DEADLINE_MS);
        options.fuel = integer(args, "fuel", MAX_FUEL);
        options.observationBudgetBytes = integer(
                args, "observation_budget_bytes", DEFAULT_OBSERVATION_BUDGET_BYTES);
        options.diagnostic = bool(args, "diagnostic", false);

        List<Operation> operations = new ArrayList<>();
        for (int index = 0; index < encodedOperations.length(); index++) {
            JSONObject encoded = encodedOperations.optJSONObject(index);
            if (encoded == null) {
                throw new BridgeServer.BridgeError(
                        "E_ARGS", "plan operation " + index + " must be an object");
            }
            String id = string(encoded, "id", true);
            String kind = string(encoded, "op", true);
            rejectUnknown(encoded, fieldsFor(kind), "plan operation " + id);
            Operation operation = new Operation(id, kind);
            operation.target = string(encoded, "target", false);
            operation.selector = string(encoded, "selector", false);
            operation.text = string(encoded, "text", false);
            operation.direction = stringOrDefault(encoded, "direction", "forward");
            operation.thenId = string(encoded, "then", false);
            operation.elseId = string(encoded, "else", false);
            operation.onError = stringOrDefault(encoded, "on_error", "stop");
            operation.errorTarget = string(encoded, "error_target", false);
            operation.observe = string(encoded, "observe", false);
            operation.observeNotVisible = bool(encoded, "observe_not", false);
            operation.timeoutMs = integer(encoded, "timeout_ms", 3_000);
            operation.observeTimeoutMs = integer(encoded, "observe_timeout_ms", 3_000);
            JSONArray dependencies = encoded.optJSONArray("depends_on");
            if (encoded.has("depends_on") && dependencies == null) {
                throw new BridgeServer.BridgeError("E_ARGS", "depends_on must be an array");
            }
            if (dependencies != null) {
                for (int dependency = 0; dependency < dependencies.length(); dependency++) {
                    Object value = dependencies.opt(dependency);
                    if (!(value instanceof String)) {
                        throw new BridgeServer.BridgeError(
                                "E_ARGS", "dependency ids must be strings");
                    }
                    operation.dependencies.add((String) value);
                }
            }
            operations.add(operation);
        }
        return preflight(operations, options);
    }

    static CompiledPlan preflight(List<Operation> source, Options options) throws Exception {
        if (source == null || source.isEmpty() || source.size() > MAX_OPERATIONS) {
            throw new BridgeServer.BridgeError(
                    "E_LIMIT", "plan.run requires 1.." + MAX_OPERATIONS + " operations");
        }
        if (options == null
                || options.deadlineMs < 1
                || options.deadlineMs > MAX_DEADLINE_MS) {
            throw new BridgeServer.BridgeError(
                    "E_LIMIT", "plan deadline must be 1.." + MAX_DEADLINE_MS + " ms");
        }
        if (options.fuel < 1 || options.fuel > MAX_FUEL) {
            throw new BridgeServer.BridgeError(
                    "E_LIMIT", "plan fuel must be 1.." + MAX_FUEL);
        }
        if (options.observationBudgetBytes < 0
                || options.observationBudgetBytes > MAX_OBSERVATION_BUDGET_BYTES) {
            throw new BridgeServer.BridgeError(
                    "E_LIMIT", "observation budget must be 0.."
                            + MAX_OBSERVATION_BUDGET_BYTES + " bytes");
        }

        List<Operation> operations = new ArrayList<>(source.size());
        Map<String, Integer> indexes = new HashMap<>();
        int stringBytes = 0;
        for (int index = 0; index < source.size(); index++) {
            Operation operation = source.get(index);
            if (operation == null) {
                throw new BridgeServer.BridgeError("E_ARGS", "plan operation must not be null");
            }
            validateId(operation.id, "operation id");
            if (indexes.put(operation.id, index) != null) {
                throw new BridgeServer.BridgeError(
                        "E_ARGS", "duplicate plan operation id " + operation.id);
            }
            validateKind(operation.kind);
            validateOperationShape(operation);
            stringBytes = addStrings(stringBytes, operation);
            operations.add(operation.copy());
        }

        for (int index = 0; index < operations.size(); index++) {
            Operation operation = operations.get(index);
            Set<String> seenDependencies = new HashSet<>();
            for (String dependency : operation.dependencies) {
                validateId(dependency, "dependency id");
                if (!seenDependencies.add(dependency)) {
                    throw new BridgeServer.BridgeError(
                            "E_ARGS", "duplicate dependency " + dependency);
                }
                Integer dependencyIndex = indexes.get(dependency);
                if (dependencyIndex == null || dependencyIndex >= index) {
                    throw new BridgeServer.BridgeError(
                            "E_ARGS", "dependency must reference an earlier operation: " + dependency);
                }
            }
            validateForwardTarget(operation.thenId, "then", index, indexes);
            validateForwardTarget(operation.elseId, "else", index, indexes);
            if ("jump".equals(operation.onError)) {
                validateForwardTarget(operation.errorTarget, "error_target", index, indexes);
                if (operation.errorTarget.isEmpty()) {
                    throw new BridgeServer.BridgeError(
                            "E_ARGS", "on_error=jump requires error_target");
                }
            } else if (!operation.errorTarget.isEmpty()) {
                throw new BridgeServer.BridgeError(
                        "E_ARGS", "error_target requires on_error=jump");
            }
        }
        return new CompiledPlan(
                Collections.unmodifiableList(operations),
                Collections.unmodifiableMap(indexes),
                copyOptions(options));
    }

    static ExecutionResult executeCompiled(
            CompiledPlan plan,
            Ui ui,
            Clock clock,
            Cancellation cancellation) throws Exception {
        if (plan == null || ui == null || clock == null || cancellation == null) {
            throw new BridgeServer.BridgeError("E_ARGS", "plan execution dependencies are missing");
        }
        // Re-run structural validation before any Ui call. Tests and future
        // in-process callers cannot bypass the same no-mutation preflight used
        // by the JSON entrypoint.
        CompiledPlan checked = preflight(plan.operations, plan.options);
        ExecutionResult result = new ExecutionResult(checked.operations.size());
        ObservationBudget observations = new ObservationBudget(
                checked.options.observationBudgetBytes);
        Fuel fuel = new Fuel(checked.options.fuel);
        long started = clock.nanoTime();
        long deadline = addNanos(started, checked.options.deadlineMs);
        State lastState = null;

        int index = 0;
        while (index < checked.operations.size()) {
            if (result.receipts.get(index) != null) {
                index++;
                continue;
            }
            Operation operation = checked.operations.get(index);
            Receipt receipt = new Receipt(operation);
            result.receipts.set(index, receipt);
            long operationStarted = clock.nanoTime();

            String unmetDependency = unmetDependency(operation, checked, result);
            if (!unmetDependency.isEmpty()) {
                receipt.status = "skipped";
                receipt.reason = "dependency:" + unmetDependency;
                result.skippedOperations++;
                receipt.elapsedMs = elapsedMillis(operationStarted, clock.nanoTime());
                receipt.state = observations.reuse(lastState);
                index++;
                continue;
            }

            int next = index + 1;
            receipt.accepted = true;
            try {
                switch (operation.kind) {
                    case "tap":
                        guardEvaluation(clock, cancellation, deadline, fuel);
                        ui.planTap(operation.target);
                        receipt.committed = true;
                        result.committedMutations++;
                        observeMutation(operation, ui, clock, cancellation, deadline, fuel);
                        receipt.observed = !operation.observe.isEmpty();
                        receipt.status = receipt.observed ? "observed" : "committed";
                        break;
                    case "text":
                        guardEvaluation(clock, cancellation, deadline, fuel);
                        ui.planText(operation.target, operation.text);
                        receipt.committed = true;
                        result.committedMutations++;
                        observeMutation(operation, ui, clock, cancellation, deadline, fuel);
                        receipt.observed = !operation.observe.isEmpty();
                        receipt.status = receipt.observed ? "observed" : "committed";
                        break;
                    case "scroll":
                        guardEvaluation(clock, cancellation, deadline, fuel);
                        ui.planScroll(operation.target, operation.direction);
                        receipt.committed = true;
                        result.committedMutations++;
                        observeMutation(operation, ui, clock, cancellation, deadline, fuel);
                        receipt.observed = !operation.observe.isEmpty();
                        receipt.status = receipt.observed ? "observed" : "committed";
                        break;
                    case "back":
                        guardEvaluation(clock, cancellation, deadline, fuel);
                        ui.planBack();
                        receipt.committed = true;
                        result.committedMutations++;
                        observeMutation(operation, ui, clock, cancellation, deadline, fuel);
                        receipt.observed = !operation.observe.isEmpty();
                        receipt.status = receipt.observed ? "observed" : "committed";
                        break;
                    case "wait.visible":
                        waitCondition(
                                ui, operation.selector, true, "E_TIMEOUT",
                                operation.timeoutMs, clock, cancellation, deadline, fuel);
                        receipt.observed = true;
                        receipt.status = "observed";
                        break;
                    case "assert.visible":
                        waitCondition(
                                ui, operation.selector, true, "E_ASSERT",
                                operation.timeoutMs, clock, cancellation, deadline, fuel);
                        receipt.observed = true;
                        receipt.status = "observed";
                        break;
                    case "assert.notVisible":
                        waitCondition(
                                ui, operation.selector, false, "E_ASSERT",
                                operation.timeoutMs, clock, cancellation, deadline, fuel);
                        receipt.observed = true;
                        receipt.status = "observed";
                        break;
                    case "if":
                        guardEvaluation(clock, cancellation, deadline, fuel);
                        boolean visible = ui.planVisible(operation.selector);
                        receipt.observed = true;
                        receipt.status = "observed";
                        String branch = visible ? operation.thenId : operation.elseId;
                        if (!branch.isEmpty()) {
                            next = checked.indexes.get(branch);
                            markSkipped(
                                    checked, result, index + 1, next,
                                    "branch:" + operation.id, lastState, observations);
                        }
                        break;
                    case "stop":
                        guardEvaluation(clock, cancellation, deadline, fuel);
                        receipt.status = "accepted";
                        result.stopped = true;
                        markSkipped(
                                checked, result, index + 1, checked.operations.size(),
                                "stopped:" + operation.id, lastState, observations);
                        next = checked.operations.size();
                        break;
                    default:
                        throw new BridgeServer.BridgeError(
                                "E_ARGS", "unsupported plan operation " + operation.kind);
                }
            } catch (BridgeServer.BridgeError error) {
                failReceipt(receipt, error);
                recordFailure(result, receipt, index);
                if ("continue".equals(operation.onError)) {
                    next = index + 1;
                } else if ("jump".equals(operation.onError)) {
                    next = checked.indexes.get(operation.errorTarget);
                    markSkipped(
                            checked, result, index + 1, next,
                            "error-branch:" + operation.id, lastState, observations);
                } else {
                    markSkipped(
                            checked, result, index + 1, checked.operations.size(),
                            "failure:" + operation.id, lastState, observations);
                    next = checked.operations.size();
                }
            } catch (Exception error) {
                BridgeServer.BridgeError helperError = new BridgeServer.BridgeError(
                        "E_HELPER", error.getClass().getSimpleName() + ": " + safeMessage(error));
                failReceipt(receipt, helperError);
                recordFailure(result, receipt, index);
                if ("continue".equals(operation.onError)) {
                    next = index + 1;
                } else if ("jump".equals(operation.onError)) {
                    next = checked.indexes.get(operation.errorTarget);
                    markSkipped(
                            checked, result, index + 1, next,
                            "error-branch:" + operation.id, lastState, observations);
                } else {
                    markSkipped(
                            checked, result, index + 1, checked.operations.size(),
                            "failure:" + operation.id, lastState, observations);
                    next = checked.operations.size();
                }
            } finally {
                receipt.elapsedMs = elapsedMillis(operationStarted, clock.nanoTime());
                State captured = captureState(ui);
                if (captured != null) {
                    lastState = captured;
                }
                receipt.state = observations.add(lastState);
            }
            index = next;
        }

        for (int missing = 0; missing < result.receipts.size(); missing++) {
            if (result.receipts.get(missing) == null) {
                Receipt skipped = new Receipt(checked.operations.get(missing));
                skipped.status = "skipped";
                skipped.reason = "not-reached";
                skipped.state = observations.reuse(lastState);
                result.receipts.set(missing, skipped);
                result.skippedOperations++;
            }
        }
        result.fuelUsed = fuel.used;
        result.elapsedMs = elapsedMillis(started, clock.nanoTime());
        if (lastState == null) {
            lastState = captureState(ui);
        }
        result.finalState = observations.add(lastState);
        result.observationBudgetExhausted = observations.exhausted;
        return result;
    }

    private static void observeMutation(
            Operation operation,
            Ui ui,
            Clock clock,
            Cancellation cancellation,
            long deadline,
            Fuel fuel) throws Exception {
        if (operation.observe.isEmpty()) {
            return;
        }
        waitCondition(
                ui,
                operation.observe,
                !operation.observeNotVisible,
                "E_ASSERT",
                operation.observeTimeoutMs,
                clock,
                cancellation,
                deadline,
                fuel);
    }

    private static void waitCondition(
            Ui ui,
            String selector,
            boolean expectedVisible,
            String timeoutCode,
            int timeoutMs,
            Clock clock,
            Cancellation cancellation,
            long planDeadline,
            Fuel fuel) throws Exception {
        long conditionDeadline = Math.min(planDeadline, addNanos(clock.nanoTime(), timeoutMs));
        while (true) {
            long now = clock.nanoTime();
            if (now >= conditionDeadline) {
                if (now >= planDeadline) {
                    throw new BridgeServer.BridgeError("E_TIMEOUT", "plan deadline exceeded");
                }
                throw new BridgeServer.BridgeError(
                        timeoutCode,
                        expectedVisible
                                ? "selector did not become visible"
                                : "selector remained visible");
            }
            guardEvaluation(clock, cancellation, planDeadline, fuel);
            boolean visible = ui.planVisible(selector);
            if (visible == expectedVisible) {
                return;
            }
            long remainingMs = remainingMillis(clock.nanoTime(), conditionDeadline);
            if (remainingMs <= 0L) {
                throw new BridgeServer.BridgeError(
                        timeoutCode,
                        expectedVisible
                                ? "selector did not become visible"
                                : "selector remained visible");
            }
            long generation = ui.planGeneration();
            ui.planAwaitChange(generation, Math.min(FALLBACK_POLL_MS, remainingMs));
        }
    }

    private static void guardEvaluation(
            Clock clock,
            Cancellation cancellation,
            long deadline,
            Fuel fuel) throws Exception {
        if (cancellation.cancelled() || Thread.currentThread().isInterrupted()) {
            throw new BridgeServer.BridgeError("E_CANCELLED", "plan execution cancelled");
        }
        if (clock.nanoTime() >= deadline) {
            throw new BridgeServer.BridgeError("E_TIMEOUT", "plan deadline exceeded");
        }
        fuel.consume();
    }

    private static void failReceipt(Receipt receipt, BridgeServer.BridgeError error) {
        receipt.status = "failed";
        receipt.error = error.code;
        receipt.message = capUtf8(error.getMessage() == null ? "" : error.getMessage(), MAX_ERROR_MESSAGE_BYTES);
    }

    private static void recordFailure(
            ExecutionResult result, Receipt receipt, int index) {
        result.complete = false;
        result.failedOperations++;
        if (result.error.isEmpty()) {
            result.error = receipt.error;
            result.message = receipt.message;
            result.failedId = receipt.id;
            result.failedIndex = index;
        }
    }

    private static void markSkipped(
            CompiledPlan plan,
            ExecutionResult result,
            int start,
            int end,
            String reason,
            State state,
            ObservationBudget observations) {
        for (int index = start; index < end; index++) {
            if (result.receipts.get(index) != null) {
                continue;
            }
            Receipt skipped = new Receipt(plan.operations.get(index));
            skipped.status = "skipped";
            skipped.reason = reason;
            skipped.state = observations.reuse(state);
            result.receipts.set(index, skipped);
            result.skippedOperations++;
        }
    }

    private static String unmetDependency(
            Operation operation,
            CompiledPlan plan,
            ExecutionResult result) {
        for (String dependency : operation.dependencies) {
            Receipt receipt = result.receipts.get(plan.indexes.get(dependency));
            if (receipt == null || !receipt.successful()) {
                return dependency;
            }
        }
        return "";
    }

    private static State captureState(Ui ui) {
        try {
            return ui.planState();
        } catch (Exception ignored) {
            return null;
        }
    }

    private static JSONObject encode(ExecutionResult result, boolean diagnostic) throws Exception {
        JSONArray receipts = new JSONArray();
        for (Receipt receipt : result.receipts) {
            receipts.put(diagnostic ? encodeDiagnostic(receipt) : encodeCompact(receipt));
        }
        JSONObject encoded = new JSONObject()
                .put("v", 1)
                .put("c", result.complete)
                .put("p", result.committedMutations > 0 || result.failedIndex > 0)
                .put("stopped", result.stopped)
                .put("m", result.committedMutations)
                .put("failed", result.failedOperations)
                .put("skipped", result.skippedOperations)
                .put("fuel", result.fuelUsed)
                .put("elapsed_ms", result.elapsedMs)
                .put("observation_budget_exhausted", result.observationBudgetExhausted)
                .put("r", receipts);
        if (result.finalState != null) {
            encoded.put("state", encodeState(result.finalState));
        }
        if (!result.error.isEmpty()) {
            encoded.put("e", result.error)
                    .put("message", result.message)
                    .put("failed_id", result.failedId)
                    .put("failed_index", result.failedIndex);
        }
        if (utf8Bytes(encoded.toString()) > FrameCodec.MAX_FRAME) {
            throw new BridgeServer.BridgeError("E_OUTPUT_LIMIT", "plan receipts exceed helper frame limit");
        }
        return encoded;
    }

    private static Object encodeCompact(Receipt receipt) throws Exception {
        int flags = (receipt.accepted ? 1 : 0)
                | (receipt.committed ? 2 : 0)
                | (receipt.observed ? 4 : 0);
        return new JSONArray()
                .put(receipt.id)
                .put(receipt.operation)
                .put(receipt.status)
                .put(flags)
                .put(receipt.elapsedMs)
                .put(receipt.state == null ? JSONObject.NULL : encodeState(receipt.state))
                .put(receipt.error.isEmpty() ? JSONObject.NULL : receipt.error)
                .put(receipt.message.isEmpty() ? JSONObject.NULL : receipt.message)
                .put(receipt.reason.isEmpty() ? JSONObject.NULL : receipt.reason);
    }

    private static Object encodeDiagnostic(Receipt receipt) throws Exception {
        JSONObject encoded = new JSONObject()
                .put("id", receipt.id)
                .put("operation", receipt.operation)
                .put("status", receipt.status)
                .put("accepted", receipt.accepted)
                .put("committed", receipt.committed)
                .put("observed", receipt.observed)
                .put("elapsed_ms", receipt.elapsedMs);
        if (receipt.state != null) {
            encoded.put("state", new JSONObject()
                    .put("generation", receipt.state.generation)
                    .put("window", receipt.state.window)
                    .put("hash", receipt.state.hash));
        }
        if (!receipt.error.isEmpty()) {
            encoded.put("error", receipt.error).put("message", receipt.message);
        }
        if (!receipt.reason.isEmpty()) {
            encoded.put("reason", receipt.reason);
        }
        return encoded;
    }

    private static JSONArray encodeState(State state) {
        return new JSONArray().put(state.generation).put(state.window).put(state.hash);
    }

    private static void validateKind(String kind) throws Exception {
        switch (kind) {
            case "tap":
            case "text":
            case "scroll":
            case "back":
            case "wait.visible":
            case "assert.visible":
            case "assert.notVisible":
            case "if":
            case "stop":
                return;
            default:
                throw new BridgeServer.BridgeError(
                        "E_ARGS", "unsupported plan operation " + kind);
        }
    }

    private static void validateOperationShape(Operation operation) throws Exception {
        if (!("stop".equals(operation.onError)
                || "continue".equals(operation.onError)
                || "jump".equals(operation.onError))) {
            throw new BridgeServer.BridgeError(
                    "E_ARGS", "on_error must be stop, continue, or jump");
        }
        if (operation.timeoutMs < 1 || operation.timeoutMs > MAX_DEADLINE_MS
                || operation.observeTimeoutMs < 1
                || operation.observeTimeoutMs > MAX_DEADLINE_MS) {
            throw new BridgeServer.BridgeError(
                    "E_LIMIT", "operation timeout must be 1.." + MAX_DEADLINE_MS + " ms");
        }
        if (!operation.observe.isEmpty()) {
            validateSelector(operation.observe, "mutation observation");
        } else if (operation.observeNotVisible) {
            throw new BridgeServer.BridgeError(
                    "E_ARGS", "observe_not requires an observe selector");
        }
        switch (operation.kind) {
            case "tap":
                validateTarget(operation.target);
                break;
            case "text":
                validateTarget(operation.target);
                requireMaxUtf8(operation.text, MAX_TEXT_BYTES, "text");
                break;
            case "scroll":
                validateTarget(operation.target);
                if (!("forward".equals(operation.direction)
                        || "backward".equals(operation.direction))) {
                    throw new BridgeServer.BridgeError(
                            "E_ARGS", "scroll direction must be forward or backward");
                }
                break;
            case "back":
            case "stop":
                break;
            case "wait.visible":
            case "assert.visible":
            case "assert.notVisible":
                validateSelector(operation.selector, operation.kind);
                break;
            case "if":
                validateSelector(operation.selector, "if selector");
                if (operation.thenId.isEmpty() && operation.elseId.isEmpty()) {
                    throw new BridgeServer.BridgeError(
                            "E_ARGS", "if requires then and/or else target");
                }
                break;
            default:
                validateKind(operation.kind);
        }
    }

    private static int addStrings(int bytes, Operation operation) throws Exception {
        int total = bytes;
        total = checkedAdd(total, utf8Bytes(operation.id));
        total = checkedAdd(total, utf8Bytes(operation.kind));
        total = checkedAdd(total, utf8Bytes(operation.target));
        total = checkedAdd(total, utf8Bytes(operation.selector));
        total = checkedAdd(total, utf8Bytes(operation.text));
        total = checkedAdd(total, utf8Bytes(operation.direction));
        total = checkedAdd(total, utf8Bytes(operation.thenId));
        total = checkedAdd(total, utf8Bytes(operation.elseId));
        total = checkedAdd(total, utf8Bytes(operation.onError));
        total = checkedAdd(total, utf8Bytes(operation.errorTarget));
        total = checkedAdd(total, utf8Bytes(operation.observe));
        for (String dependency : operation.dependencies) {
            total = checkedAdd(total, utf8Bytes(dependency));
        }
        if (total > MAX_TOTAL_STRING_BYTES) {
            throw new BridgeServer.BridgeError(
                    "E_LIMIT", "plan strings exceed " + MAX_TOTAL_STRING_BYTES + " bytes");
        }
        return total;
    }

    private static int checkedAdd(int left, int right) throws Exception {
        if (left > MAX_TOTAL_STRING_BYTES - right) {
            throw new BridgeServer.BridgeError(
                    "E_LIMIT", "plan strings exceed " + MAX_TOTAL_STRING_BYTES + " bytes");
        }
        return left + right;
    }

    private static void validateTarget(String target) throws Exception {
        requireUtf8(target, MAX_SELECTOR_BYTES, "semantic target");
        if (target.matches("-?[0-9]{1,20}")) {
            try {
                Long.parseLong(target);
                return;
            } catch (NumberFormatException error) {
                throw new BridgeServer.BridgeError("E_ARGS", "numeric semantic target is out of range");
            }
        }
        if (target.matches("s[0-9a-z]{1,32}")) {
            return;
        }
        validateSelector(target, "semantic target");
    }

    private static void validateSelector(String selector, String label) throws Exception {
        requireUtf8(selector, MAX_SELECTOR_BYTES, label);
        String body = selector;
        int marker = lastUnescaped(body, '#');
        if (marker >= 0) {
            String occurrence = body.substring(marker + 1);
            if (occurrence.isEmpty() || occurrence.length() > 6 || !occurrence.matches("[0-9]+")) {
                throw new BridgeServer.BridgeError("E_ARGS", label + " has invalid occurrence");
            }
            body = body.substring(0, marker);
        }
        List<String> terms = splitEscaped(body, ',');
        if (terms.isEmpty() || terms.size() > 16) {
            throw new BridgeServer.BridgeError("E_ARGS", label + " has invalid selector terms");
        }
        for (String term : terms) {
            String trimmed = term.trim();
            int contains = firstUnescaped(trimmed, '~');
            int equals = firstUnescaped(trimmed, '=');
            int split = contains >= 0 ? contains : equals;
            if (split < 1 || split == trimmed.length() - 1) {
                throw new BridgeServer.BridgeError("E_ARGS", label + " has invalid selector syntax");
            }
            String field = trimmed.substring(0, split).trim().toLowerCase();
            if (!("text".equals(field) || "desc".equals(field) || "id".equals(field)
                    || "class".equals(field) || "pkg".equals(field)
                    || "clickable".equals(field) || "enabled".equals(field)
                    || "scrollable".equals(field) || "checked".equals(field)
                    || "bounds".equals(field))) {
                throw new BridgeServer.BridgeError("E_ARGS", label + " has unknown selector field");
            }
            if ("clickable".equals(field) || "enabled".equals(field)
                    || "scrollable".equals(field) || "checked".equals(field)) {
                String value = unescape(trimmed.substring(split + 1).trim());
                if (contains >= 0 || !("true".equals(value) || "false".equals(value))) {
                    throw new BridgeServer.BridgeError(
                            "E_ARGS", label + " has invalid boolean selector");
                }
            }
        }
    }

    private static void validateId(String id, String label) throws Exception {
        requireUtf8(id, MAX_ID_BYTES, label);
        if (!id.matches("[A-Za-z0-9][A-Za-z0-9._:-]{0,63}")) {
            throw new BridgeServer.BridgeError("E_ARGS", label + " has invalid characters");
        }
    }

    private static void validateForwardTarget(
            String id,
            String label,
            int sourceIndex,
            Map<String, Integer> indexes) throws Exception {
        if (id.isEmpty()) {
            return;
        }
        validateId(id, label);
        Integer target = indexes.get(id);
        if (target == null) {
            throw new BridgeServer.BridgeError("E_ARGS", label + " target does not exist: " + id);
        }
        if (target <= sourceIndex) {
            throw new BridgeServer.BridgeError(
                    "E_ARGS", label + " target must be forward-only: " + id);
        }
    }

    private static Set<String> fieldsFor(String kind) throws Exception {
        validateKind(kind);
        Set<String> fields = allowed(
                "id", "op", "depends_on", "on_error", "error_target",
                "observe", "observe_not", "observe_timeout_ms");
        switch (kind) {
            case "tap":
                fields.add("target");
                break;
            case "text":
                fields.add("target");
                fields.add("text");
                break;
            case "scroll":
                fields.add("target");
                fields.add("direction");
                break;
            case "wait.visible":
            case "assert.visible":
            case "assert.notVisible":
                fields.add("selector");
                fields.add("timeout_ms");
                break;
            case "if":
                fields.add("selector");
                fields.add("then");
                fields.add("else");
                break;
            case "back":
            case "stop":
                break;
            default:
                throw new BridgeServer.BridgeError(
                        "E_ARGS", "unsupported plan operation " + kind);
        }
        return fields;
    }

    private static void rejectUnknown(JSONObject object, Set<String> allowed, String label)
            throws Exception {
        Iterator<String> keys = object.keys();
        while (keys.hasNext()) {
            String key = keys.next();
            if (!allowed.contains(key)) {
                throw new BridgeServer.BridgeError(
                        "E_ARGS", label + " has unknown field " + key);
            }
        }
    }

    private static Set<String> allowed(String... values) {
        Set<String> result = new HashSet<>();
        Collections.addAll(result, values);
        return result;
    }

    private static String string(JSONObject object, String key, boolean required) throws Exception {
        if (!object.has(key)) {
            if (required) {
                throw new BridgeServer.BridgeError("E_ARGS", key + " is required");
            }
            return "";
        }
        Object value = object.opt(key);
        if (!(value instanceof String)) {
            throw new BridgeServer.BridgeError("E_ARGS", key + " must be a string");
        }
        String result = (String) value;
        if (required && result.isEmpty()) {
            throw new BridgeServer.BridgeError("E_ARGS", key + " must not be empty");
        }
        return result;
    }

    private static String stringOrDefault(JSONObject object, String key, String fallback)
            throws Exception {
        return object.has(key) ? string(object, key, false) : fallback;
    }

    private static int integer(JSONObject object, String key, int fallback) throws Exception {
        if (!object.has(key)) {
            return fallback;
        }
        Object value = object.opt(key);
        if (!(value instanceof Number)) {
            throw new BridgeServer.BridgeError("E_ARGS", key + " must be an integer");
        }
        Number number = (Number) value;
        double encoded = number.doubleValue();
        long integral = number.longValue();
        if (!Double.isFinite(encoded) || encoded != integral
                || integral < Integer.MIN_VALUE || integral > Integer.MAX_VALUE) {
            throw new BridgeServer.BridgeError("E_ARGS", key + " must be an integer");
        }
        return (int) integral;
    }

    private static boolean bool(JSONObject object, String key, boolean fallback) throws Exception {
        if (!object.has(key)) {
            return fallback;
        }
        Object value = object.opt(key);
        if (!(value instanceof Boolean)) {
            throw new BridgeServer.BridgeError("E_ARGS", key + " must be a boolean");
        }
        return (Boolean) value;
    }

    private static void requireUtf8(String value, int maximum, String label) throws Exception {
        int bytes = utf8Bytes(value == null ? "" : value);
        if (bytes < 1 || bytes > maximum) {
            throw new BridgeServer.BridgeError(
                    "E_LIMIT", label + " must be 1.." + maximum + " UTF-8 bytes");
        }
    }

    private static void requireMaxUtf8(String value, int maximum, String label) throws Exception {
        int bytes = utf8Bytes(value == null ? "" : value);
        if (bytes > maximum) {
            throw new BridgeServer.BridgeError(
                    "E_LIMIT", label + " must be at most " + maximum + " UTF-8 bytes");
        }
    }

    private static int utf8Bytes(String value) {
        return value.getBytes(StandardCharsets.UTF_8).length;
    }

    private static String capUtf8(String value, int maximum) {
        if (utf8Bytes(value) <= maximum) {
            return value;
        }
        StringBuilder result = new StringBuilder();
        for (int offset = 0; offset < value.length();) {
            int codePoint = value.codePointAt(offset);
            String next = new String(Character.toChars(codePoint));
            if (utf8Bytes(result.toString()) + utf8Bytes(next) > maximum) {
                break;
            }
            result.append(next);
            offset += Character.charCount(codePoint);
        }
        return result.toString();
    }

    private static Options copyOptions(Options source) {
        Options copy = new Options();
        copy.deadlineMs = source.deadlineMs;
        copy.fuel = source.fuel;
        copy.observationBudgetBytes = source.observationBudgetBytes;
        copy.diagnostic = source.diagnostic;
        return copy;
    }

    private static long addNanos(long start, long milliseconds) {
        long delta = milliseconds * 1_000_000L;
        if (Long.MAX_VALUE - start < delta) {
            return Long.MAX_VALUE;
        }
        return start + delta;
    }

    private static long elapsedMillis(long start, long end) {
        return Math.max(0L, (end - start) / 1_000_000L);
    }

    private static long remainingMillis(long now, long deadline) {
        if (now >= deadline) {
            return 0L;
        }
        long nanos = deadline - now;
        return Math.max(1L, (nanos + 999_999L) / 1_000_000L);
    }

    private static String safeMessage(Exception error) {
        String message = error.getMessage();
        return message == null ? "" : capUtf8(message, MAX_ERROR_MESSAGE_BYTES);
    }

    private static int firstUnescaped(String value, char needle) {
        boolean escaped = false;
        for (int index = 0; index < value.length(); index++) {
            char current = value.charAt(index);
            if (escaped) {
                escaped = false;
            } else if (current == '\\') {
                escaped = true;
            } else if (current == needle) {
                return index;
            }
        }
        return -1;
    }

    private static int lastUnescaped(String value, char needle) {
        boolean escaped = false;
        int result = -1;
        for (int index = 0; index < value.length(); index++) {
            char current = value.charAt(index);
            if (escaped) {
                escaped = false;
            } else if (current == '\\') {
                escaped = true;
            } else if (current == needle) {
                result = index;
            }
        }
        return result;
    }

    private static List<String> splitEscaped(String value, char delimiter) {
        List<String> parts = new ArrayList<>();
        StringBuilder current = new StringBuilder();
        boolean escaped = false;
        for (int index = 0; index < value.length(); index++) {
            char character = value.charAt(index);
            if (escaped) {
                current.append('\\').append(character);
                escaped = false;
            } else if (character == '\\') {
                escaped = true;
            } else if (character == delimiter) {
                parts.add(current.toString());
                current.setLength(0);
            } else {
                current.append(character);
            }
        }
        if (escaped) {
            current.append('\\');
        }
        parts.add(current.toString());
        return parts;
    }

    private static String unescape(String value) {
        StringBuilder result = new StringBuilder();
        boolean escaped = false;
        for (int index = 0; index < value.length(); index++) {
            char current = value.charAt(index);
            if (escaped) {
                result.append(current);
                escaped = false;
            } else if (current == '\\') {
                escaped = true;
            } else {
                result.append(current);
            }
        }
        if (escaped) {
            result.append('\\');
        }
        return result.toString();
    }

    private static final class Fuel {
        final int limit;
        int used;

        Fuel(int limit) {
            this.limit = limit;
        }

        void consume() throws Exception {
            if (used >= limit) {
                throw new BridgeServer.BridgeError("E_LIMIT", "plan evaluation fuel exhausted");
            }
            used++;
        }
    }

    private static final class ObservationBudget {
        final int limit;
        int used;
        boolean exhausted;

        ObservationBudget(int limit) {
            this.limit = limit;
        }

        State add(State state) {
            if (state == null) {
                return null;
            }
            int encoded = 32 + utf8Bytes(state.hash);
            if (encoded > limit - used) {
                exhausted = true;
                return null;
            }
            used += encoded;
            return state;
        }

        State reuse(State state) {
            return add(state);
        }
    }
}
