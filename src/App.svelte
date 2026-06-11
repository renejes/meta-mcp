<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { open } from "@tauri-apps/plugin-dialog";
  import { api } from "./lib/api";
  import type {
    ClaudeStatus,
    Config,
    ProxyStatus,
    ServerEntry,
    ServerStatus,
    ToolWithServer,
  } from "./lib/types";
  import Icon from "./lib/Icon.svelte";
  import ProfileBar from "./lib/ProfileBar.svelte";
  import ServerList from "./lib/ServerList.svelte";
  import ToolList from "./lib/ToolList.svelte";
  import AddServerModal from "./lib/AddServerModal.svelte";
  import ClaudeConnect from "./lib/ClaudeConnect.svelte";

  let config = $state<Config | null>(null);
  let proxyStatus = $state<ProxyStatus | null>(null);
  let statuses = $state<ServerStatus[]>([]);
  let tools = $state<ToolWithServer[]>([]);
  let claudeStatus = $state<ClaudeStatus>({ code: false, desktop: false });

  let editingServer = $state<ServerEntry | null>(null);
  let showModal = $state(false);
  let importPreview = $state<ServerEntry[] | null>(null);
  let error = $state<string | null>(null);

  const profileActive = $derived(!!config?.active_profile);

  async function refresh() {
    try {
      config = await api.getConfig();
      statuses = await api.getServerStatus();
      tools = await api.getToolList();
      claudeStatus = await api.getClaudeStatus();
    } catch (e) {
      error = String(e);
    }
  }

  const toggleClaudeCode = (enabled: boolean) =>
    run(() => api.setClaudeCode(enabled))();
  const toggleClaudeDesktop = (enabled: boolean) =>
    run(() => api.setClaudeDesktop(enabled))();

  function run(action: () => Promise<unknown>) {
    return async () => {
      try {
        await action();
        await refresh();
      } catch (e) {
        error = String(e);
      }
    };
  }

  onMount(async () => {
    try {
      proxyStatus = await api.getProxyStatus();
    } catch {
      /* ignore */
    }
    await refresh();
    await listen<ProxyStatus>("proxy-status-changed", (e) => {
      proxyStatus = e.payload;
      refresh();
    });
  });

  // ---- server actions ----
  function openAdd() {
    editingServer = null;
    showModal = true;
  }
  function openEdit(server: ServerEntry) {
    editingServer = server;
    showModal = true;
  }
  async function handleSaveServer(entry: ServerEntry) {
    showModal = false;
    await run(() => api.saveServer(entry))();
  }
  const toggleServer = (id: string, active: boolean) =>
    run(() => api.setServerActive(id, active))();
  const deleteServer = (id: string) => run(() => api.deleteServer(id))();

  // ---- profile actions ----
  const selectProfile = (id: string | null) =>
    run(() => api.setActiveProfile(id))();
  async function saveProfile(name: string) {
    const activeIds = statuses.filter((s) => s.active).map((s) => s.id);
    await run(() =>
      api.saveProfile({ id: "", name, active_server_ids: activeIds }),
    )();
  }
  const deleteProfile = (id: string) => run(() => api.deleteProfile(id))();

  // ---- import ----
  async function startImport() {
    try {
      let defaultPath: string | undefined;
      try {
        defaultPath = await api.defaultClaudeConfigPath();
      } catch {
        /* ignore */
      }
      const selected = await open({
        title: "claude_desktop_config.json auswählen",
        defaultPath,
        multiple: false,
        directory: false,
        filters: [{ name: "JSON", extensions: ["json"] }],
      });
      if (typeof selected !== "string") return;
      const entries = await api.importClaudeConfig(selected);
      if (entries.length === 0) {
        error = "Keine neuen Server gefunden (oder alle bereits vorhanden).";
        return;
      }
      importPreview = entries;
    } catch (e) {
      error = String(e);
    }
  }
  async function confirmImport() {
    const entries = importPreview ?? [];
    importPreview = null;
    for (const entry of entries) {
      try {
        await api.saveServer(entry);
      } catch (e) {
        error = String(e);
      }
    }
    await refresh();
  }

  const statusUi = $derived.by(() => {
    switch (proxyStatus?.state) {
      case "running":
        return { dot: "bg-ok", glow: "shadow-[0_0_8px_2px_rgba(52,211,153,0.45)]", pulse: false };
      case "error":
        return { dot: "bg-err", glow: "shadow-[0_0_8px_2px_rgba(248,113,113,0.45)]", pulse: false };
      default:
        return { dot: "bg-warn", glow: "", pulse: true };
    }
  });
</script>

<header
  class="sticky top-0 z-10 flex items-center justify-between gap-3 border-b border-line bg-surface/90 px-5 py-3 backdrop-blur"
>
  <div class="min-w-0">
    {#if config}
      <ProfileBar
        {config}
        onSelect={selectProfile}
        onSave={saveProfile}
        onDelete={deleteProfile}
      />
    {/if}
  </div>

  <div class="flex items-center gap-3">
    <div class="flex items-center gap-2">
      <div
        class="grid h-7 w-7 place-items-center rounded-lg bg-gradient-to-br from-brand to-brand-hi shadow-lg shadow-brand/30"
      >
        <Icon name="hub" size={15} class="text-white" />
      </div>
      <span class="font-semibold tracking-tight">Meta-MCP</span>
    </div>

    <div
      class="flex items-center gap-2 rounded-full border border-line bg-surface-2 py-1 pl-2.5 pr-3"
      title={proxyStatus?.message ?? ""}
    >
      <span class="relative grid place-items-center">
        <span class="h-2.5 w-2.5 rounded-full {statusUi.dot} {statusUi.glow}"></span>
        {#if statusUi.pulse}
          <span
            class="absolute h-2.5 w-2.5 animate-ping rounded-full {statusUi.dot} opacity-60"
          ></span>
        {/if}
      </span>
      <span class="max-w-[220px] truncate text-xs text-muted"
        >{proxyStatus?.message ?? "…"}</span
      >
    </div>
  </div>
</header>

{#if error}
  <div
    class="flex items-center justify-between gap-3 border-b border-err/30 bg-err/10 px-5 py-2 text-sm text-err"
  >
    <span class="flex items-center gap-2 min-w-0">
      <Icon name="error" size={18} class="shrink-0" />
      <span class="truncate">{error}</span>
    </span>
    <button
      class="grid h-7 w-7 shrink-0 place-items-center rounded-md text-err/80 hover:bg-err/15 hover:text-err"
      onclick={() => (error = null)}
      aria-label="Schließen"
    >
      <Icon name="close" size={18} />
    </button>
  </div>
{/if}

<main class="mx-auto max-w-3xl space-y-8 px-5 py-7">
  {#if config}
    <ClaudeConnect
      status={claudeStatus}
      onToggleCode={toggleClaudeCode}
      onToggleDesktop={toggleClaudeDesktop}
    />
    <ServerList
      servers={config.servers}
      {statuses}
      {profileActive}
      onAdd={openAdd}
      onImport={startImport}
      onToggle={toggleServer}
      onEdit={openEdit}
      onDelete={deleteServer}
    />
    <ToolList {tools} />
  {:else}
    <div class="flex items-center gap-3 py-16 text-muted">
      <Icon name="progress_activity" size={20} class="animate-spin" />
      <span>Lädt…</span>
    </div>
  {/if}
</main>

{#if showModal}
  <AddServerModal
    server={editingServer}
    onSave={handleSaveServer}
    onClose={() => (showModal = false)}
  />
{/if}

{#if importPreview}
  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
  <div
    class="fixed inset-0 z-50 grid place-items-center bg-black/60 p-5 backdrop-blur-sm"
    role="presentation"
    onclick={() => (importPreview = null)}
  >
    <!-- svelte-ignore a11y_click_events_have_key_events a11y_interactive_supports_focus -->
    <div
      class="w-full max-w-lg rounded-2xl border border-line-strong bg-surface p-6 shadow-2xl"
      role="dialog"
      tabindex="-1"
      onclick={(e) => e.stopPropagation()}
    >
      <div class="mb-1 flex items-center gap-2">
        <Icon name="file_download" size={20} class="text-brand-hi" />
        <h2 class="text-base font-semibold">Import-Vorschau</h2>
      </div>
      <p class="text-sm text-muted">
        {importPreview.length} neue Server werden hinzugefügt (inaktiv):
      </p>
      <div class="mt-3 flex max-h-[50vh] flex-col gap-1.5 overflow-auto">
        {#each importPreview as entry (entry.id)}
          <div
            class="flex items-center gap-2.5 rounded-lg bg-surface-2 px-3 py-2 text-sm"
          >
            <span
              class="rounded-md border border-line-strong px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-muted"
              >{entry.transport}</span
            >
            <strong class="font-medium">{entry.name}</strong>
            <span class="truncate font-mono text-xs text-faint">
              {entry.transport === "stdio"
                ? `${entry.command ?? ""} ${(entry.args ?? []).join(" ")}`
                : (entry.url ?? "")}
            </span>
          </div>
        {/each}
      </div>
      <div class="mt-5 flex justify-end gap-2.5">
        <button
          class="rounded-lg px-3.5 py-2 text-sm text-muted hover:bg-surface-2 hover:text-fg"
          onclick={() => (importPreview = null)}>Abbrechen</button
        >
        <button
          class="flex items-center gap-1.5 rounded-lg bg-brand px-3.5 py-2 text-sm font-medium text-white transition hover:bg-brand-hi"
          onclick={confirmImport}
        >
          <Icon name="download" size={17} />
          Importieren
        </button>
      </div>
    </div>
  </div>
{/if}
