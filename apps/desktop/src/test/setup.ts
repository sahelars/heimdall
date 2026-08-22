import "@testing-library/jest-dom/vitest";
import { vi } from "vitest";

/*
 * jsdom stops short of several browser APIs this application relies on. Each
 * stub below stands in for one of them; none of them changes what is being
 * tested, they only stop the environment from throwing before the assertion.
 */

// Theme resolution asks the platform what it prefers.
Object.defineProperty(window, "matchMedia", {
  writable: true,
  value: (query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    addListener: vi.fn(),
    removeListener: vi.fn(),
    dispatchEvent: vi.fn(),
  }),
});

// The graph canvas and the panes size themselves from their container.
class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}
vi.stubGlobal("ResizeObserver", ResizeObserverStub);

// jsdom has no canvas. The graph pane already treats a null context as "draw
// nothing", so returning null exercises that path instead of filling the output
// with "not implemented" warnings.
HTMLCanvasElement.prototype.getContext = (() => null) as HTMLCanvasElement["getContext"];

// jsdom implements <dialog> but not its modal methods, so the settings modal's
// own open/close logic is still exercised against these.
HTMLDialogElement.prototype.showModal = function showModal() {
  this.setAttribute("open", "");
};
HTMLDialogElement.prototype.close = function close() {
  this.removeAttribute("open");
  this.dispatchEvent(new Event("close"));
};
