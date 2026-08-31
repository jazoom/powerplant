import {
    bindTransportFeedback,
    listenForDiagnostics,
    startHypergraft,
} from "hypergraft/browser";
import { initComposer, initShortcutHint } from "./composer";
import { initConnectErrors } from "./connect-errors";
import { initConnectPlan } from "./connect-plan";
import { initDeskSettings } from "./desk-settings";
import { initObserve } from "./observe";
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
        "connect-plan": initConnectPlan,
        "desk-settings": initDeskSettings,
        observe: initObserve,
        "shortcut-hint": initShortcutHint,
        transcript: initTranscript,
    },
});
