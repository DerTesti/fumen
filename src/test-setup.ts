import { vi } from "vitest";

// English text assertions must not depend on the developer's system language.
// German dictionary completeness is covered separately by i18n.test.ts.
vi.stubGlobal("navigator", { language: "en-GB" });
