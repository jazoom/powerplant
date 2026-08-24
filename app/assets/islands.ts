import { observeIslands } from "hypergraft/browser/islands";
import { initComposer } from "./composer";
import { initConnectErrors } from "./connect-errors";
import { initJobObserve } from "./job-observe";
import { initTranscript } from "./transcript";

const islands = {
    composer: initComposer,
    "connect-errors": initConnectErrors,
    "job-observe": initJobObserve,
    transcript: initTranscript,
};

if (document.readyState === "loading")
    document.addEventListener(
        "DOMContentLoaded",
        () => observeIslands(islands),
        { once: true },
    );
else observeIslands(islands);
