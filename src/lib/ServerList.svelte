<script lang="ts">
  import type { ServerEntry, ServerStatus } from "./types";
  import Icon from "./Icon.svelte";

  let {
    servers,
    statuses,
    profileActive,
    onAdd,
    onImport,
    onToggle,
    onEdit,
    onDelete,
  }: {
    servers: ServerEntry[];
    statuses: ServerStatus[];
    profileActive: boolean;
    onAdd: () => void;
    onImport: () => void;
    onToggle: (id: string, active: boolean) => void;
    onEdit: (server: ServerEntry) => void;
    onDelete: (id: string) => void;
  } = $props();

  let menuOpen = $state<string | null>(null);

  const statusById = $derived(
    new Map(statuses.map((s) => [s.id, s] as const)),
  );

  function dotClass(s: ServerStatus | undefined): string {
    if (!s || !s.active) return "bg-faint";
    return s.connected ? "bg-ok shadow-[0_0_7px_1px_rgba(52,211,153,0.45)]" : "bg-err";
  }
</script>

<section>
  <div class="mb-3 flex items-center justify-between">
    <div class="flex items-center gap-2">
      <h2 class="text-sm font-semibold tracking-tight">Server</h2>
      <span class="chip">{servers.length}</span>
    </div>
    <div class="flex items-center gap-2">
      <button class="btn btn-ghost" onclick={onImport}>
        <Icon name="upload_file" size={18} />
        Import aus Claude Config
      </button>
      <button class="btn btn-primary" onclick={onAdd}>
        <Icon name="add" size={18} />
        Server
      </button>
    </div>
  </div>

  {#if profileActive}
    <div
      class="mb-3 flex items-center gap-2 rounded-lg border border-warn/30 bg-warn/10 px-3 py-2 text-xs text-warn"
    >
      <Icon name="lock" size={16} class="shrink-0" />
      Ein Profil ist aktiv — die Auswahl bestimmt das Profil. Toggles sind
      deaktiviert.
    </div>
  {/if}

  {#if servers.length === 0}
    <div
      class="flex flex-col items-center gap-2 rounded-xl border border-dashed border-line-strong px-6 py-12 text-center"
    >
      <Icon name="dns" size={34} class="text-faint" />
      <p class="text-sm text-muted">
        Noch keine Server. Füge einen hinzu oder importiere aus deiner Claude
        Config.
      </p>
    </div>
  {:else}
    <div class="space-y-2">
      {#each servers as server (server.id)}
        {@const s = statusById.get(server.id)}
        <div
          class="card flex items-center gap-3 px-4 py-3 transition-colors hover:border-line-strong"
        >
          <span class="h-2.5 w-2.5 shrink-0 rounded-full {dotClass(s)}"></span>

          <div class="min-w-0 flex-1">
            <div class="flex items-center gap-2">
              <span class="truncate font-medium">{server.name}</span>
              <span class="chip gap-1">
                <Icon
                  name={server.transport === "stdio" ? "terminal" : "cloud"}
                  size={12}
                />
                {server.transport}
              </span>
            </div>
            <div
              class="mt-0.5 flex items-center gap-1 text-xs {s &&
              s.active &&
              !s.connected
                ? 'text-err'
                : 'text-muted'}"
            >
              {#if !s || !s.active}
                <Icon name="radio_button_unchecked" size={14} />
                inaktiv
              {:else if !s.connected}
                <Icon name="warning" size={14} />
                Verbindungsfehler
              {:else}
                <Icon name="build" size={14} />
                {s.tool_count}
                {s.tool_count === 1 ? "Tool" : "Tools"}
              {/if}
            </div>
          </div>

          <label
            class="relative inline-flex shrink-0 cursor-pointer items-center"
            title={profileActive ? "Vom Profil gesteuert" : "Aktiv schalten"}
          >
            <input
              type="checkbox"
              class="peer sr-only"
              checked={s?.active ?? false}
              disabled={profileActive}
              onchange={(e) =>
                onToggle(server.id, (e.target as HTMLInputElement).checked)}
            />
            <div
              class="h-[22px] w-[38px] rounded-full bg-line-strong transition-colors peer-checked:bg-brand peer-disabled:opacity-40"
            ></div>
            <div
              class="pointer-events-none absolute left-[3px] h-4 w-4 rounded-full bg-white shadow transition-transform peer-checked:translate-x-4"
            ></div>
          </label>

          <div class="relative shrink-0">
            <button
              class="btn-icon"
              onclick={() =>
                (menuOpen = menuOpen === server.id ? null : server.id)}
              aria-label="Menü"
            >
              <Icon name="more_vert" size={18} />
            </button>
            {#if menuOpen === server.id}
              <div
                class="absolute right-0 top-full z-20 mt-1 w-36 overflow-hidden rounded-lg border border-line-strong bg-surface-2 py-1 shadow-xl"
              >
                <button
                  class="flex w-full items-center gap-2 px-3 py-2 text-left text-sm text-fg hover:bg-surface-3"
                  onclick={() => {
                    menuOpen = null;
                    onEdit(server);
                  }}
                >
                  <Icon name="edit" size={17} class="text-muted" />
                  Bearbeiten
                </button>
                <button
                  class="flex w-full items-center gap-2 px-3 py-2 text-left text-sm text-err hover:bg-err/10"
                  onclick={() => {
                    menuOpen = null;
                    onDelete(server.id);
                  }}
                >
                  <Icon name="delete" size={17} />
                  Löschen
                </button>
              </div>
            {/if}
          </div>
        </div>
      {/each}
    </div>
  {/if}

  {#if menuOpen}
    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
    <div
      class="fixed inset-0 z-10"
      role="presentation"
      onclick={() => (menuOpen = null)}
    ></div>
  {/if}
</section>
