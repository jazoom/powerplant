import {
    bindTransportFeedback,
    listenForDiagnostics,
    startHypergraft,
} from "hypergraft/browser";
import { initComposer, initShortcutHint } from "./composer";
import { initConnectErrors } from "./connect-errors";
import { initJobObserve } from "./job-observe";
import { initTranscript } from "./transcript";

const bound = bindTransportFeedback(document);

if (import.meta.env.DEV) {
    listenForDiagnostics((detail) => {
        console.error("Hypergraft diagnostic", detail);
    });
}

startHypergraft({
    feedback: bound.feedback,
    islands: {
        composer: initComposer,
        "connect-errors": initConnectErrors,
        "job-observe": initJobObserve,
        "shortcut-hint": initShortcutHint,
        transcript: initTranscript,
    },
});
