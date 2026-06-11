<script lang="ts">
  import type { ClaudeStatus } from "./types";
  import Icon from "./Icon.svelte";

  let {
    status,
    onToggleCode,
    onToggleDesktop,
  }: {
    status: ClaudeStatus;
    onToggleCode: (enabled: boolean) => void;
    onToggleDesktop: (enabled: boolean) => void;
  } = $props();

  const targets = $derived([
    {
      key: "code" as const,
      icon: "code",
      name: "Claude Code",
      sub: "type/url → http://localhost:3663/mcp",
      connected: status.code,
      onToggle: onToggleCode,
    },
    {
      key: "desktop" as const,
      icon: "computer",
      name: "Claude Desktop",
      sub: "stdio-Bridge (meta-mcp --stdio)",
      connected: status.desktop,
      onToggle: onToggleDesktop,
    },
  ]);
</script>

<section>
  <div class="mb-3 flex items-center gap-2">
    <h2 class="text-sm font-semibold tracking-tight">Claude-Anbindung</h2>
  </div>

  <div class="card divide-y divide-line">
    {#each targets as t (t.key)}
      <div class="flex items-center gap-3 px-4 py-3">
        <div class="grid h-8 w-8 shrink-0 place-items-center rounded-lg bg-surface-3">
          <Icon name={t.icon} size={18} class="text-brand-hi" />
        </div>
        <div class="min-w-0 flex-1">
          <div class="font-medium">{t.name}</div>
          <div
            class="mt-0.5 flex items-center gap-1 text-xs {t.connected
              ? 'text-ok'
              : 'text-muted'}"
          >
            <Icon
              name={t.connected ? "check_circle" : "remove_circle"}
              size={14}
            />
            {t.connected ? "Verbunden" : "Nicht verbunden"}
            <span class="ml-1.5 truncate font-mono text-faint">· {t.sub}</span>
          </div>
        </div>

        <label
          class="relative inline-flex shrink-0 cursor-pointer items-center"
          title={t.connected ? "Eintrag entfernen" : "In Claude eintragen"}
        >
          <input
            type="checkbox"
            class="peer sr-only"
            checked={t.connected}
            onchange={(e) =>
              t.onToggle((e.target as HTMLInputElement).checked)}
          />
          <div
            class="h-[22px] w-[38px] rounded-full bg-line-strong transition-colors peer-checked:bg-brand"
          ></div>
          <div
            class="pointer-events-none absolute left-[3px] h-4 w-4 rounded-full bg-white shadow transition-transform peer-checked:translate-x-4"
          ></div>
        </label>
      </div>
    {/each}
  </div>

  <p class="mt-2 flex items-center gap-1.5 text-xs text-faint">
    <Icon name="info" size={14} />
    Andere Apps können sich selbst eintragen:
    <code class="font-mono text-muted">POST http://localhost:3663/register</code>
  </p>
</section>
