import {
    bindTransportFeedback,
    listenForDiagnostics,
    startHypergraft,
} from "hypergraft/browser";

const bound = bindTransportFeedback(document);

if (import.meta.env.DEV) {
    listenForDiagnostics((detail) => {
        console.error("Hypergraft diagnostic", detail);
    });
}

startHypergraft({ feedback: bound.feedback });
