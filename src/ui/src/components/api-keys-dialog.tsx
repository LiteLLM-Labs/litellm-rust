"use client";

import { useEffect, useMemo, useState } from "react";
import { Check, Copy, KeyRound, Loader2, Plus, Trash2 } from "lucide-react";
import { toast } from "sonner";

import { BrandIcon } from "@/components/brand-icons";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import {
  createGatewayApiKey,
  deleteGatewayApiKey,
  listGatewayApiKeys,
  type CreatedGatewayApiKey,
  type GatewayApiKey,
} from "@/lib/api";

export function ApiKeysDialog() {
  const [open, setOpen] = useState(false);
  const [keys, setKeys] = useState<GatewayApiKey[] | null>(null);
  const [label, setLabel] = useState("");
  const [creating, setCreating] = useState(false);
  const [created, setCreated] = useState<CreatedGatewayApiKey | null>(null);

  const load = async () => {
    setKeys(await listGatewayApiKeys());
  };

  useEffect(() => {
    if (!open) return;
    load().catch((error) => toast.error(error instanceof Error ? error.message : String(error)));
  }, [open]);

  const updateOpen = (nextOpen: boolean) => {
    setOpen(nextOpen);
    if (!nextOpen) {
      setCreated(null);
      setLabel("");
    }
  };

  const create = async () => {
    setCreating(true);
    setCreated(null);
    try {
      const key = await createGatewayApiKey(label);
      setCreated(key);
      setLabel("");
      await load();
    } catch (error) {
      toast.error(error instanceof Error ? error.message : String(error));
    } finally {
      setCreating(false);
    }
  };

  const remove = async (id: string) => {
    setKeys((current) => current?.filter((key) => key.id !== id) ?? null);
    try {
      await deleteGatewayApiKey(id);
    } catch (error) {
      toast.error(error instanceof Error ? error.message : String(error));
      await load().catch(() => {});
    }
  };

  return (
    <>
      <Button
        variant="outline"
        size="sm"
        onClick={() => setOpen(true)}
        aria-label="API Key"
        title="API Key"
      >
        <KeyRound className="size-4" />
        <span className="hidden lg:inline">API Key</span>
      </Button>
      <Dialog open={open} onOpenChange={updateOpen}>
        <DialogContent className="max-h-[min(780px,calc(100vh-2rem))] overflow-y-auto sm:max-w-3xl">
          <DialogHeader>
            <DialogTitle>API Keys</DialogTitle>
            <DialogDescription>Create gateway keys for local CLIs and AI agents.</DialogDescription>
          </DialogHeader>

          <div className="grid gap-4">
            <div className="rounded-lg border border-border bg-card p-3">
              <div className="flex flex-col gap-2 sm:flex-row">
                <Input
                  value={label}
                  onChange={(event) => setLabel(event.target.value)}
                  placeholder="Label, optional"
                  onKeyDown={(event) => {
                    if (event.key === "Enter") create();
                  }}
                />
                <Button onClick={create} disabled={creating} className="shrink-0">
                  {creating ? <Loader2 className="size-4 animate-spin" /> : <Plus className="size-4" />}
                  Create API Key
                </Button>
              </div>
            </div>

            {created && <CreatedKeyCard created={created} />}

            <div className="rounded-lg border border-border">
              <div className="border-b border-border px-4 py-3 text-sm font-medium">Existing keys</div>
              {keys === null ? (
                <div className="flex items-center gap-2 px-4 py-6 text-sm text-muted-foreground">
                  <Loader2 className="size-4 animate-spin" />
                  Loading
                </div>
              ) : keys.length === 0 ? (
                <div className="px-4 py-6 text-sm text-muted-foreground">No API keys yet.</div>
              ) : (
                <div className="divide-y divide-border">
                  {keys.map((key) => (
                    <div key={key.id} className="flex items-center justify-between gap-3 px-4 py-3">
                      <div className="min-w-0">
                        <div className="truncate text-sm font-medium">{key.label || "Untitled key"}</div>
                        <div className="mt-1 truncate font-mono text-xs text-muted-foreground">
                          {key.id} - {key.last_used_at ? new Date(key.last_used_at * 1000).toLocaleString() : "never used"}
                        </div>
                      </div>
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        className="shrink-0 text-destructive hover:text-destructive"
                        onClick={() => remove(key.id)}
                        aria-label="Delete API key"
                      >
                        <Trash2 className="size-4" />
                      </Button>
                    </div>
                  ))}
                </div>
              )}
            </div>
          </div>
        </DialogContent>
      </Dialog>
    </>
  );
}

function CreatedKeyCard({ created }: { created: CreatedGatewayApiKey }) {
  const origin = typeof window === "undefined" ? "http://127.0.0.1:4000" : window.location.origin;
  const claudeCommand = `lite claude --url "${origin}" --key "${created.key}"`;
  const codexCommand = `lite codex --url "${origin}" --key "${created.key}"`;
  const agentPrompt = useMemo(
    () => `You have access to LiteLLM's Rust AI gateway at ${origin}. Ask the user for a LiteLLM API key if you need to make authenticated calls.

Start by checking what you can access:
- Providers and model IDs: GET ${origin}/v1/models
- Full gateway capabilities: GET ${origin}/api/capabilities
- OpenAPI schema and endpoints: GET ${origin}/openapi.json
- MCP servers: inspect "mcp_servers" from /api/capabilities, then call ${origin}/mcp or ${origin}/mcp/{server_id}
- Managed agents: inspect "agents" from /api/capabilities, then call POST ${origin}/api/agents/{agent_id}/run when available`,
    [origin],
  );

  return (
    <div className="grid gap-3 rounded-lg border border-border bg-card p-4">
      <div>
        <div className="text-sm font-medium">Your API key</div>
        <div className="mt-2 flex items-center gap-2 rounded-lg bg-muted px-3 py-2">
          <code className="min-w-0 flex-1 overflow-x-auto font-mono text-sm">{created.key}</code>
          <CopyButton value={created.key} label="Copy API key" />
        </div>
      </div>

      <CommandCard icon="claude" title="Start Claude Code" command={claudeCommand} />
      <CommandCard icon="codex" title="Start Codex" command={codexCommand} />

      <div className="rounded-lg border border-border p-3">
        <div className="mb-2 flex items-center justify-between gap-2">
          <div className="text-sm font-medium">Prompt for AI agents</div>
          <CopyButton value={agentPrompt} label="Copy agent prompt" />
        </div>
        <pre className="max-h-56 overflow-auto whitespace-pre-wrap rounded-lg bg-muted p-3 font-mono text-xs leading-5 text-muted-foreground">
          {agentPrompt}
        </pre>
      </div>
    </div>
  );
}

function CommandCard({ icon, title, command }: { icon: string; title: string; command: string }) {
  return (
    <div className="rounded-lg border border-border p-3">
      <div className="mb-2 flex items-center justify-between gap-2">
        <div className="flex min-w-0 items-center gap-2 text-sm font-medium">
          <BrandIcon id={icon} className="size-5" />
          <span className="truncate">{title}</span>
        </div>
        <CopyButton value={command} label={`Copy ${title} command`} />
      </div>
      <code className="block overflow-x-auto rounded-lg bg-muted px-3 py-2 font-mono text-xs">
        {command}
      </code>
    </div>
  );
}

function CopyButton({ value, label }: { value: string; label: string }) {
  const [copied, setCopied] = useState(false);

  const copy = async () => {
    await navigator.clipboard.writeText(value);
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1200);
  };

  return (
    <Button variant="ghost" size="icon-sm" onClick={copy} aria-label={label} title={label}>
      {copied ? <Check className="size-4" /> : <Copy className="size-4" />}
    </Button>
  );
}
