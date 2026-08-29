import {
    bindTransportFeedback,
    listenForDiagnostics,
    startHypergraft,
} from "hypergraft/browser";
import { initComposer, initShortcutHint } from "./composer";
import { initConnectErrors } from "./connect-errors";
import { initConnectPlan } from "./connect-plan";
import { initDeskSettings } from "./desk-settings";
import { initJobObserve } from "./job-observe";
import { initEnvironmentPreparation } from "./environment-preparation";
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
        "environment-preparation": initEnvironmentPreparation,
        "job-observe": initJobObserve,
        "shortcut-hint": initShortcutHint,
        transcript: initTranscript,
    },
});
