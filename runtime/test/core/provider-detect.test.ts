import { describe, it, expect } from "vitest";
import {
  detectProviderFromModel,
  detectProviderFromRoute,
} from "../../src/core/provider-detect.js";

describe("detectProviderFromModel", () => {
  it("detects anthropic from provider ID", () => {
    expect(detectProviderFromModel({ provider: "anthropic" })).toBe("anthropic");
  });

  it("detects anthropic from dotted provider ID", () => {
    expect(detectProviderFromModel({ provider: "anthropic.messages" })).toBe(
      "anthropic",
    );
  });

  it("detects openai from provider ID", () => {
    expect(detectProviderFromModel({ provider: "openai" })).toBe("openai");
  });

  it("detects openai from dotted provider ID", () => {
    expect(detectProviderFromModel({ provider: "openai.chat" })).toBe("openai");
  });

  it("detects google from provider ID", () => {
    expect(detectProviderFromModel({ provider: "google" })).toBe("google");
  });

  it("detects google from google-generative-ai", () => {
    expect(
      detectProviderFromModel({ provider: "google-generative-ai" }),
    ).toBe("google");
  });

  it("detects google from google-vertex", () => {
    expect(detectProviderFromModel({ provider: "google-vertex" })).toBe(
      "google",
    );
  });

  it("detects anthropic from amazon-bedrock (Claude on Bedrock)", () => {
    expect(detectProviderFromModel({ provider: "amazon-bedrock" })).toBe(
      "anthropic",
    );
  });

  it("falls back to model ID prefix: claude-*", () => {
    expect(
      detectProviderFromModel({ modelId: "claude-sonnet-4-20250514" }),
    ).toBe("anthropic");
  });

  it("falls back to model ID prefix: gpt-*", () => {
    expect(detectProviderFromModel({ modelId: "gpt-4o" })).toBe("openai");
  });

  it("falls back to model ID prefix: o1", () => {
    expect(detectProviderFromModel({ modelId: "o1" })).toBe("openai");
  });

  it("falls back to model ID prefix: o3", () => {
    expect(detectProviderFromModel({ modelId: "o3-mini" })).toBe("openai");
  });

  it("falls back to model ID prefix: gemini-*", () => {
    expect(detectProviderFromModel({ modelId: "gemini-2.5-pro" })).toBe(
      "google",
    );
  });

  it("returns undefined for unknown provider and model", () => {
    expect(
      detectProviderFromModel({ provider: "unknown", modelId: "custom-model" }),
    ).toBeUndefined();
  });

  it("returns undefined when no info provided", () => {
    expect(detectProviderFromModel({})).toBeUndefined();
  });

  it("prefers provider ID over model ID", () => {
    expect(
      detectProviderFromModel({ provider: "openai", modelId: "claude-sonnet-4-20250514" }),
    ).toBe("openai");
  });
});

describe("detectProviderFromRoute", () => {
  it("detects anthropic from /v1/messages", () => {
    expect(detectProviderFromRoute("/v1/messages")).toBe("anthropic");
  });

  it("detects openai from /v1/chat/completions", () => {
    expect(detectProviderFromRoute("/v1/chat/completions")).toBe("openai");
  });

  it("detects google from /v1beta/models/gemini-2.5-pro:generateContent", () => {
    expect(
      detectProviderFromRoute(
        "/v1beta/models/gemini-2.5-pro:generateContent",
      ),
    ).toBe("google");
  });

  it("returns undefined for unknown path", () => {
    expect(detectProviderFromRoute("/unknown/endpoint")).toBeUndefined();
  });
});
