"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { Bot, Check, KeyRound, Plus, ServerCog, X } from "lucide-react";
import { Sidebar } from "@/components/sidebar";
import { ThemeToggle } from "@/components/theme-toggle";
import { BrandIcon } from "@/components/brand-icons";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  deleteAnthropicProvider,
  listProviders,
  saveAnthropicProvider,
  type ConnectedProvider,
} from "@/lib/api";

type Step = "catalog" | "configure" | "connected";

const ANTHROPIC = {
  name: "Anthropic",
  description: "Claude models through the Anthropic Messages API",
  defaultBaseUrl: "https://api.anthropic.com",
};

export default function SettingsPage() {
  const [step, setStep] = useState<Step>("catalog");
  const [apiKey, setApiKey] = useState("");
  const [baseUrl, setBaseUrl] = useState(ANTHROPIC.defaultBaseUrl);
  const [connectedProvider, setConnectedProvider] = useState<ConnectedProvider | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const connected = Boolean(connectedProvider);

  const refreshProviders = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const data = await listProviders();
      const anthropic = data.connected_providers.find((provider) => provider.id === "anthropic");
      setConnectedProvider(anthropic ?? null);
      if (anthropic) {
        setBaseUrl(anthropic.api_base);
        setStep("connected");
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load providers");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refreshProviders();
  }, [refreshProviders]);

  const maskedKey = useMemo(() => {
    if (connectedProvider) return connectedProvider.masked_api_key;
    const trimmed = apiKey.trim();
    if (!trimmed) return "No API key";
    if (trimmed.length <= 10) return "Configured";
    return `${trimmed.slice(0, 7)}...${trimmed.slice(-4)}`;
  }, [apiKey, connectedProvider]);

  const connect = async () => {
    if (!apiKey.trim() || !baseUrl.trim()) return;
    setSaving(true);
    setError(null);
    try {
      const data = await saveAnthropicProvider({ apiKey, apiBase: baseUrl });
      const anthropic = data.connected_providers.find((provider) => provider.id === "anthropic");
      setConnectedProvider(anthropic ?? null);
      setApiKey("");
      setStep("connected");
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to save provider");
    } finally {
      setSaving(false);
    }
  };

  const disconnect = async () => {
    setSaving(true);
    setError(null);
    try {
      await deleteAnthropicProvider();
      setConnectedProvider(null);
      setApiKey("");
      setBaseUrl(ANTHROPIC.defaultBaseUrl);
      setStep("catalog");
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to disconnect provider");
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="flex h-screen bg-background text-foreground">
      <Sidebar />
      <div className="flex min-w-0 flex-1 flex-col">
        <header className="flex h-12 shrink-0 items-center justify-between border-b border-border px-4">
          <div className="flex items-center gap-2">
            <ServerCog className="size-4 text-muted-foreground" />
            <h1 className="text-sm font-semibold">Settings</h1>
          </div>
          <ThemeToggle />
        </header>

        <main className="flex-1 overflow-y-auto">
          <div className="mx-auto flex max-w-5xl flex-col gap-5 px-4 py-6">
            <div className="flex flex-col gap-1">
              <h2 className="text-lg font-semibold">AI Providers</h2>
              <p className="text-sm text-muted-foreground">
                Connect provider credentials before assigning models to agents.
              </p>
              {loading && <p className="text-xs text-muted-foreground">Loading providers...</p>}
              {error && <p className="text-xs text-destructive">{error}</p>}
            </div>

            {connected && (
              <section className="grid gap-2">
                <h3>Connected providers</h3>
                <Card className="flex items-center justify-between gap-4 p-4">
                  <div className="flex min-w-0 items-center gap-3">
                    <ProviderLogo />
                    <div className="min-w-0">
                      <div className="flex flex-wrap items-center gap-2">
                        <span className="font-medium">{ANTHROPIC.name}</span>
                        <Badge variant="secondary" className="text-[10px]">
                          API key
                        </Badge>
                        <Badge variant="outline" className="text-[10px]">
                          {connectedProvider?.api_base ?? baseUrl}
                        </Badge>
                      </div>
                      <p className="mt-1 font-mono text-xs text-muted-foreground">{maskedKey}</p>
                    </div>
                  </div>
                  <Button variant="outline" size="sm" onClick={disconnect} disabled={saving}>
                    <X className="size-3.5" />
                    Disconnect
                  </Button>
                </Card>
              </section>
            )}

            <section className="grid gap-2">
              <div className="flex items-center justify-between gap-3">
                <h3>Available providers</h3>
                <Badge variant="outline" className="text-[10px]">
                  Rust proxy catalog
                </Badge>
              </div>
              <Card className="overflow-hidden p-0">
                <button
                  type="button"
                  className="flex w-full items-center justify-between gap-4 px-4 py-4 text-left transition-colors hover:bg-muted/50"
                  onClick={() => {
                    setBaseUrl(connectedProvider?.api_base ?? ANTHROPIC.defaultBaseUrl);
                    setStep("configure");
                  }}
                >
                  <div className="flex min-w-0 items-center gap-3">
                    <ProviderLogo />
                    <div className="min-w-0">
                      <div className="flex flex-wrap items-center gap-2">
                        <span className="font-medium">{ANTHROPIC.name}</span>
                        <Badge variant="secondary" className="text-[10px]">
                          Available
                        </Badge>
                      </div>
                      <p className="mt-1 text-sm text-muted-foreground">
                        {ANTHROPIC.description}
                      </p>
                    </div>
                  </div>
                  <span className="inline-flex h-7 shrink-0 items-center justify-center gap-1 rounded-lg border border-border bg-background px-2.5 text-[0.8rem] font-medium shadow-sm">
                    <Plus className="size-3.5" />
                    Connect
                  </span>
                </button>
              </Card>
            </section>

            {step !== "catalog" && (
              <section className="grid gap-2">
                <h3>{connected ? "Provider details" : "Connect Anthropic"}</h3>
                <Card className="p-4">
                  <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_280px]">
                    <div className="grid gap-4">
                      <div className="flex items-center gap-3">
                        <ProviderLogo large />
                        <div>
                          <div className="font-medium">{ANTHROPIC.name}</div>
                          <p className="text-sm text-muted-foreground">
                            Add your Anthropic API key and base URL.
                          </p>
                        </div>
                      </div>

                      <div className="grid gap-1.5">
                        <Label htmlFor="anthropic-key">Anthropic API key</Label>
                        <div className="relative">
                          <KeyRound className="absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
                          <Input
                            id="anthropic-key"
                            type="password"
                            value={apiKey}
                            onChange={(event) => setApiKey(event.target.value)}
                            placeholder="sk-ant-..."
                            className="pl-8 font-mono text-xs"
                          />
                        </div>
                      </div>

                      <div className="grid gap-1.5">
                        <Label htmlFor="anthropic-base-url">Anthropic base URL</Label>
                        <Input
                          id="anthropic-base-url"
                          value={baseUrl}
                          onChange={(event) => setBaseUrl(event.target.value)}
                          placeholder={ANTHROPIC.defaultBaseUrl}
                          className="font-mono text-xs"
                        />
                      </div>
                    </div>

                    <div className="rounded-lg border border-border bg-muted/30 p-3">
                      <div className="flex items-center gap-2 text-sm font-medium">
                        <Bot className="size-4" />
                        Agent routing
                      </div>
                      <div className="mt-3 space-y-3 text-xs text-muted-foreground">
                        <div className="flex items-center justify-between gap-3 border-b border-border pb-2">
                          <span>Provider</span>
                          <span className="text-foreground">{ANTHROPIC.name}</span>
                        </div>
                        <div className="flex items-center justify-between gap-3 border-b border-border pb-2">
                          <span>Models</span>
                          <span className="font-mono text-foreground">anthropic/*</span>
                        </div>
                        <div className="flex items-center justify-between gap-3">
                          <span>Status</span>
                          <span className="inline-flex items-center gap-1 text-foreground">
                            {connected && <Check className="size-3" />}
                            {connected ? "Connected" : "Ready"}
                          </span>
                        </div>
                      </div>
                    </div>
                  </div>

                  <div className="mt-4 flex justify-end gap-2">
                    <Button variant="outline" size="sm" onClick={() => setStep("catalog")}>
                      Cancel
                    </Button>
                    <Button
                      size="sm"
                      onClick={connect}
                      disabled={saving || !apiKey.trim() || !baseUrl.trim()}
                    >
                      <Check className="size-3.5" />
                      {saving ? "Saving..." : "Save provider"}
                    </Button>
                  </div>
                </Card>
              </section>
            )}
          </div>
        </main>
      </div>
    </div>
  );
}

function ProviderLogo({ large = false }: { large?: boolean }) {
  return (
    <span
      className={`flex shrink-0 items-center justify-center rounded-md border border-border bg-background text-foreground shadow-sm ${
        large ? "size-11" : "size-9"
      }`}
    >
      <BrandIcon id="anthropic" className={large ? "size-7" : "size-5"} />
    </span>
  );
}
