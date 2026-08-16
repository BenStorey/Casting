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

describe("SetupWizard", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // Mock global fetch for the policy POST calls inside launch()
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
    expect(screen.getByText("Welcome to Casting")).toBeTruthy();
    expect(screen.getByText("Get started")).toBeTruthy();
  });

  it("calls submitSetup with the user's data when launched", async () => {
    const mockSubmit = vi.mocked(submitSetup);
    mockSubmit.mockResolvedValueOnce({ ok: true, hires: [], objective: "" });
    const onDone = vi.fn();

    render(<SetupWizard onDone={onDone} />);

    // Navigate through all 9 wizard steps

    // Step 1: Welcome → Name
    fireEvent.click(findButton("Get started"));

    // Step 2: Name → Experience
    await screen.findByText("What should I call you?");
    fireEvent.change(screen.getByPlaceholderText("e.g. Ben"), {
      target: { value: "Test User" },
    });
    fireEvent.click(findButton("Continue"));

    // Step 3: Experience → Cast intro
    await screen.findByText("How familiar are you with software development?");
    fireEvent.click(screen.getByText("Confident with technology"));
    fireEvent.click(findButton("Continue"));

    // Step 4: Cast intro (1 member) → Existing project
    await screen.findByText("Alex Engineer");
    fireEvent.click(findButton("All set"));

    // Step 5: Existing project → Project details
    await screen.findByText("Do you have an existing project?");
    fireEvent.click(screen.getByText("Start something new"));
    fireEvent.click(findButton("Continue"));

    // Step 6: Project details → Policies
    await screen.findByText("Tell me about your project");
    fireEvent.change(screen.getByPlaceholderText("e.g. MyTodo"), {
      target: { value: "MyTestProject" },
    });
    fireEvent.change(screen.getByPlaceholderText(/e\.g\. A todo app/), {
      target: { value: "A test project" },
    });
    fireEvent.click(findButton("Continue"));

    // Step 7: Policies → API Key
    await screen.findByText("How much autonomy should the PM have?");
    fireEvent.click(findButton("Continue"));

    // Step 8: API Key → Launch
    await screen.findByText("Connect your AI provider");
    fireEvent.change(screen.getByPlaceholderText("sk-or-v1-..."), {
      target: { value: "sk-or-v1-test123" },
    });
    fireEvent.click(findButton("Continue"));

    // Step 9: Launch
    await screen.findByText("Ready to launch");
    expect(screen.getByText("Test User")).toBeTruthy();
    expect(screen.getByText("MyTestProject")).toBeTruthy();

    fireEvent.click(findButton("Launch my company"));

    // Verify submitSetup was called with the right arguments
    await waitFor(() => {
      expect(mockSubmit).toHaveBeenCalledTimes(1);
    });
    const args = mockSubmit.mock.calls[0];
    expect(args[0]).toBe("MyTestProject"); // project name
    expect(args[1]).toBe("A test project"); // objective
    expect(args[3]).toBe("Test User");      // owner name
    expect(args[4]).toBe("confident");      // exp level
    expect(args[5]).toBe("sk-or-v1-test123"); // api key

    await waitFor(() => {
      expect(onDone).toHaveBeenCalledTimes(1);
    });
  });

  it("shows an error when submitSetup fails", async () => {
    const mockSubmit = vi.mocked(submitSetup);
    mockSubmit.mockRejectedValueOnce(new Error("Network error"));
    const onDone = vi.fn();

    render(<SetupWizard onDone={onDone} />);

    // Navigate to launch
    fireEvent.click(findButton("Get started"));
    await screen.findByText("What should I call you?");
    fireEvent.change(screen.getByPlaceholderText("e.g. Ben"), {
      target: { value: "User" },
    });
    fireEvent.click(findButton("Continue"));

    await screen.findByText("How familiar are you with software development?");
    fireEvent.click(screen.getByText("Confident with technology"));
    fireEvent.click(findButton("Continue"));

    await screen.findByText("Alex Engineer");
    fireEvent.click(findButton("All set"));

    await screen.findByText("Do you have an existing project?");
    fireEvent.click(screen.getByText("Start something new"));
    fireEvent.click(findButton("Continue"));

    await screen.findByText("Tell me about your project");
    fireEvent.change(screen.getByPlaceholderText("e.g. MyTodo"), {
      target: { value: "P" },
    });
    fireEvent.change(screen.getByPlaceholderText(/e\.g\. A todo app/), {
      target: { value: "P" },
    });
    fireEvent.click(findButton("Continue"));

    await screen.findByText("How much autonomy should the PM have?");
    fireEvent.click(findButton("Continue"));

    await screen.findByText("Connect your AI provider");
    fireEvent.change(screen.getByPlaceholderText("sk-or-v1-..."), {
      target: { value: "key" },
    });
    fireEvent.click(findButton("Continue"));

    await screen.findByText("Ready to launch");
    fireEvent.click(findButton("Launch my company"));

    // Error should be displayed
    await waitFor(() => {
      expect(screen.getByText(/Network error/)).toBeTruthy();
    });
    expect(onDone).not.toHaveBeenCalled();
  });
});