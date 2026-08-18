import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor, cleanup } from "@testing-library/react";
import SetupWizard from "../SetupWizard";

// Mock API module — inline vi.fn() so no hoisting issues
vi.mock("../api", () => ({
  fetchSetupStatus: vi.fn().mockResolvedValue({
    configured: false,
    roles: [],
  }),
  submitSetup: vi.fn(),
}));

import { submitSetup } from "../api";

vi.mock("../store", () => ({
  useCastStore: vi.fn((selector: any) => {
    const store = {
      consultants: [
        {
          id: "engineer-1",
          name: "Alex Engineer",
          title: "Software Engineer",
          role: "engineer",
          avatar: null,
          summary: "Builds things",
          routing: { specializations: ["rust"] },
        },
      ],
    };
    return selector(store);
  }),
}));

vi.mock("@/components/ui/button", () => ({
  Button: ({ children, onClick, disabled, ...props }: any) => (
    <button onClick={onClick} disabled={disabled} data-testid="button" {...props}>
      {children}
    </button>
  ),
}));

vi.mock("@/components/ui/input", () => ({
  Input: ({ onChange, value, placeholder, ...props }: any) => (
    <input
      onChange={onChange}
      value={value}
      placeholder={placeholder}
      data-testid="input"
      {...props}
    />
  ),
}));

vi.mock("@/components/ui/card", () => ({
  Card: ({ children, ...props }: any) => (
    <div data-testid="card" {...props}>{children}</div>
  ),
  CardContent: ({ children, ...props }: any) => (
    <div data-testid="card-content" {...props}>{children}</div>
  ),
}));

vi.mock("@/components/ui/badge", () => ({
  Badge: ({ children }: any) => <span data-testid="badge">{children}</span>,
}));

// Helper: find a button by its visible text content
function findButton(text: string): HTMLElement {
  const buttons = screen.getAllByTestId("button");
  return buttons.find((b) => b.textContent?.includes(text))!;
}

async function walkToLaunch() {
  // Step 1: About you (welcome + name + experience)
  await screen.findByText(/Welcome to Casting/);
  fireEvent.change(screen.getByPlaceholderText("e.g. Ben"), {
    target: { value: "Test User" },
  });
  fireEvent.click(screen.getByText("Confident with technology"));
  fireEvent.click(findButton("Continue"));

  // Step 2: Meet your cast (1 member) → Continue
  await screen.findByText("Alex Engineer");
  fireEvent.click(findButton("Continue"));

  // Step 3: Your project
  await screen.findByRole("heading", { name: /Your project/i });
  fireEvent.click(screen.getByText("Start something new"));
  fireEvent.change(screen.getByPlaceholderText("e.g. MyTodo"), {
    target: { value: "MyTestProject" },
  });
  fireEvent.change(screen.getByPlaceholderText(/e\.g\. A todo app/), {
    target: { value: "A test project" },
  });
  fireEvent.click(findButton("Continue"));

  // Step 4: Autonomy
  await screen.findByText(/How much autonomy/);
  fireEvent.click(findButton("Continue"));

  // Step 5: AI provider
  await screen.findByText(/Connect your AI provider/);
  fireEvent.change(screen.getByPlaceholderText("sk-or-v1-..."), {
    target: { value: "«redacted:sk-…»" },
  });
  fireEvent.click(findButton("Continue"));

  // Step 6: Launch
  await screen.findByText("Ready to launch");
}

describe("SetupWizard", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({ ok: true, json: async () => ({}) })
    );
  });

  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
  });

  it("renders the welcome screen initially", () => {
    render(<SetupWizard onDone={() => {}} />);
    expect(screen.getByText(/Welcome to Casting/)).toBeTruthy();
  });

  it("calls submitSetup with the user's data when launched", async () => {
    const mockSubmit = vi.mocked(submitSetup);
    mockSubmit.mockResolvedValueOnce({ ok: true, hires: [], objective: "" });
    const onDone = vi.fn();

    render(<SetupWizard onDone={onDone} />);

    await walkToLaunch();
    expect(screen.getByText("Test User")).toBeTruthy();
    expect(screen.getByText("MyTestProject")).toBeTruthy();

    fireEvent.click(findButton("Launch my company"));

    await waitFor(() => {
      expect(mockSubmit).toHaveBeenCalledTimes(1);
    });
    const args = mockSubmit.mock.calls[0];
    expect(args[0]).toBe("MyTestProject"); // project name
    expect(args[1]).toBe("A test project"); // objective
    expect(args[3]).toBe("Test User");      // owner name
    expect(args[4]).toBe("confident");      // exp level
    expect(args[5]).toBe("«redacted:sk-…»"); // api key

    await waitFor(() => {
      expect(onDone).toHaveBeenCalledTimes(1);
    });
  });

  it("shows an error when submitSetup fails", async () => {
    const mockSubmit = vi.mocked(submitSetup);
    mockSubmit.mockRejectedValueOnce(new Error("Network error"));
    const onDone = vi.fn();

    render(<SetupWizard onDone={onDone} />);

    await walkToLaunch();
    fireEvent.click(findButton("Launch my company"));

    await waitFor(() => {
      expect(screen.getByText(/Network error/)).toBeTruthy();
    });
    expect(onDone).not.toHaveBeenCalled();
  });
});
