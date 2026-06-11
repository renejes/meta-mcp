<script lang="ts">
  import type { ToolWithServer } from "./types";
  import Icon from "./Icon.svelte";

  let { tools }: { tools: ToolWithServer[] } = $props();

  let expanded = $state(true);
  let filter = $state("");

  const filtered = $derived(
    tools.filter((t) => {
      const q = filter.trim().toLowerCase();
      if (!q) return true;
      return (
        t.name.toLowerCase().includes(q) ||
        t.server_name.toLowerCase().includes(q) ||
        (t.description ?? "").toLowerCase().includes(q)
      );
    }),
  );
</script>

<section>
  <button
    class="group mb-3 flex w-full items-center gap-2"
    onclick={() => (expanded = !expanded)}
  >
    <Icon
      name="chevron_right"
      size={20}
      class="text-muted transition-transform duration-150 {expanded
        ? 'rotate-90'
        : ''}"
    />
    <h2 class="text-sm font-semibold tracking-tight">Tools</h2>
    <span class="chip">{tools.length} aktiv</span>
  </button>

  {#if expanded}
    <div class="relative mb-3">
      <Icon
        name="search"
        size={18}
        class="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-faint"
      />
      <input class="field-input pl-9" placeholder="Tools filtern…" bind:value={filter} />
    </div>

    {#if filtered.length === 0}
      <div
        class="flex flex-col items-center gap-2 rounded-xl border border-dashed border-line-strong px-6 py-10 text-center"
      >
        <Icon name="build" size={30} class="text-faint" />
        <p class="text-sm text-muted">
          {tools.length === 0
            ? "Keine aktiven Tools. Aktiviere einen Server."
            : "Keine Treffer."}
        </p>
      </div>
    {:else}
      <div class="card divide-y divide-line">
        {#each filtered as tool (tool.name)}
          <div
            class="grid grid-cols-[minmax(160px,1.1fr)_auto_minmax(0,1.6fr)] items-center gap-3 px-4 py-2.5"
          >
            <code class="truncate font-mono text-[12.5px] text-brand-hi"
              >{tool.name}</code
            >
            <span class="chip justify-self-start gap-1">
              <Icon name="dns" size={11} />
              {tool.server_name}
            </span>
            <span class="truncate text-xs text-faint">{tool.description ?? ""}</span>
          </div>
        {/each}
      </div>
    {/if}
  {/if}
</section>
