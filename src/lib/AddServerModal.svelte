<script lang="ts">
  import { untrack } from "svelte";
  import type { ServerEntry, Transport } from "./types";
  import Icon from "./Icon.svelte";

  let {
    server,
    onSave,
    onClose,
  }: {
    server: ServerEntry | null;
    onSave: (entry: ServerEntry) => void;
    onClose: () => void;
  } = $props();

  // The modal is mounted fresh per open, so we seed the form from the prop once.
  const initial = untrack(() => server);
  const editing = !!initial;

  let name = $state(initial?.name ?? "");
  let transport = $state<Transport>(initial?.transport ?? "stdio");
  let command = $state(initial?.command ?? "");
  let args = $state<string[]>(initial?.args ? [...initial.args] : []);
  let url = $state(initial?.url ?? "");
  let envPairs = $state<{ key: string; value: string }[]>(
    initial?.env
      ? Object.entries(initial.env).map(([key, value]) => ({ key, value }))
      : [],
  );

  let attempted = $state(false);

  const nameError = $derived(name.trim() === "");
  const commandError = $derived(transport === "stdio" && command.trim() === "");
  const urlError = $derived(
    transport !== "stdio" && !/^https?:\/\/.+/i.test(url.trim()),
  );
  const invalid = $derived(
    nameError || (transport === "stdio" ? commandError : urlError),
  );

  function addArg() {
    args = [...args, ""];
  }
  function removeArg(i: number) {
    args = args.filter((_, idx) => idx !== i);
  }
  function addEnv() {
    envPairs = [...envPairs, { key: "", value: "" }];
  }
  function removeEnv(i: number) {
    envPairs = envPairs.filter((_, idx) => idx !== i);
  }

  function save() {
    attempted = true;
    if (invalid) return;

    const entry: ServerEntry = {
      id: initial?.id ?? "",
      name: name.trim(),
      transport,
      active: initial?.active ?? false,
    };

    if (transport === "stdio") {
      entry.command = command.trim();
      const cleanArgs = args.map((a) => a.trim()).filter((a) => a !== "");
      if (cleanArgs.length) entry.args = cleanArgs;
      const env: Record<string, string> = {};
      for (const { key, value } of envPairs) {
        if (key.trim()) env[key.trim()] = value;
      }
      if (Object.keys(env).length) entry.env = env;
    } else {
      entry.url = url.trim();
    }

    onSave(entry);
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
<div
  class="fixed inset-0 z-50 grid place-items-center bg-black/60 p-5 backdrop-blur-sm"
  role="presentation"
  onclick={onClose}
>
  <!-- svelte-ignore a11y_click_events_have_key_events a11y_interactive_supports_focus -->
  <div
    class="max-h-[88vh] w-full max-w-lg overflow-auto rounded-2xl border border-line-strong bg-surface p-6 shadow-2xl"
    role="dialog"
    aria-modal="true"
    tabindex="-1"
    onclick={(e) => e.stopPropagation()}
  >
    <div class="mb-5 flex items-center gap-2">
      <Icon
        name={editing ? "edit" : "add_circle"}
        size={20}
        class="text-brand-hi"
      />
      <h2 class="text-base font-semibold">
        {editing ? "Server bearbeiten" : "Server hinzufügen"}
      </h2>
    </div>

    <div class="mb-4">
      <label for="srv-name" class="mb-1.5 block text-xs text-muted">Name</label>
      <input id="srv-name" class="field-input" bind:value={name} placeholder="z.B. GitHub MCP" />
      {#if attempted && nameError}
        <span class="mt-1.5 flex items-center gap-1 text-xs text-err">
          <Icon name="error" size={14} />Name darf nicht leer sein.
        </span>
      {/if}
    </div>

    <div class="mb-4">
      <span class="mb-1.5 block text-xs text-muted">Transport</span>
      <div class="grid grid-cols-3 gap-1 rounded-lg border border-line-strong bg-ink p-1">
        {#each [{ v: "stdio", icon: "terminal", label: "stdio" }, { v: "http", icon: "cloud", label: "HTTP" }, { v: "sse", icon: "cloud", label: "SSE" }] as opt (opt.v)}
          <button
            type="button"
            class="flex items-center justify-center gap-1.5 rounded-md py-1.5 text-sm transition-colors {transport ===
            opt.v
              ? 'bg-surface-3 text-fg shadow-sm'
              : 'text-muted hover:text-fg'}"
            onclick={() => (transport = opt.v as Transport)}
          >
            <Icon name={opt.icon} size={16} />
            {opt.label}
          </button>
        {/each}
      </div>
    </div>

    {#if transport === "stdio"}
      <div class="mb-4">
        <label for="srv-cmd" class="mb-1.5 block text-xs text-muted">Command</label>
        <input id="srv-cmd" class="field-input" bind:value={command} placeholder="z.B. npx" />
        {#if attempted && commandError}
          <span class="mt-1.5 flex items-center gap-1 text-xs text-err">
            <Icon name="error" size={14} />Command darf nicht leer sein.
          </span>
        {/if}
      </div>

      <div class="mb-4">
        <span class="mb-1.5 block text-xs text-muted">Args</span>
        <div class="space-y-1.5">
          {#each args as _, i (i)}
            <div class="flex gap-2">
              <input class="field-input font-mono" bind:value={args[i]} placeholder="Argument" />
              <button class="btn-icon shrink-0 hover:text-err" onclick={() => removeArg(i)} aria-label="Entfernen">
                <Icon name="close" size={18} />
              </button>
            </div>
          {/each}
        </div>
        <button class="btn btn-ghost mt-1.5 px-2 py-1 text-xs" onclick={addArg}>
          <Icon name="add" size={16} />Argument
        </button>
      </div>

      <div class="mb-4">
        <span class="mb-1.5 block text-xs text-muted">Env-Variablen</span>
        <div class="space-y-1.5">
          {#each envPairs as pair, i (i)}
            <div class="flex gap-2">
              <input class="field-input font-mono" bind:value={pair.key} placeholder="KEY" />
              <input class="field-input font-mono" bind:value={pair.value} placeholder="value" />
              <button class="btn-icon shrink-0 hover:text-err" onclick={() => removeEnv(i)} aria-label="Entfernen">
                <Icon name="close" size={18} />
              </button>
            </div>
          {/each}
        </div>
        <button class="btn btn-ghost mt-1.5 px-2 py-1 text-xs" onclick={addEnv}>
          <Icon name="add" size={16} />Variable
        </button>
      </div>
    {:else}
      <div class="mb-4">
        <label for="srv-url" class="mb-1.5 block text-xs text-muted">URL</label>
        <input id="srv-url" class="field-input font-mono" bind:value={url} placeholder={transport === "http" ? "http://localhost:8080/mcp" : "http://localhost:8080/sse"} />
        {#if attempted && urlError}
          <span class="mt-1.5 flex items-center gap-1 text-xs text-err">
            <Icon name="error" size={14} />Bitte eine gültige http(s)-URL angeben.
          </span>
        {/if}
      </div>
    {/if}

    <div class="mt-6 flex justify-end gap-2.5">
      <button class="btn btn-ghost" onclick={onClose}>Abbrechen</button>
      <button class="btn btn-primary" onclick={save}>
        <Icon name="save" size={18} />Speichern
      </button>
    </div>
  </div>
</div>
