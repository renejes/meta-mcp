<script lang="ts">
  import type { Config } from "./types";
  import Icon from "./Icon.svelte";

  let {
    config,
    onSelect,
    onSave,
    onDelete,
  }: {
    config: Config;
    onSelect: (id: string | null) => void;
    onSave: (name: string) => void;
    onDelete: (id: string) => void;
  } = $props();

  let naming = $state(false);
  let newName = $state("");

  function handleSelect(e: Event) {
    const value = (e.target as HTMLSelectElement).value;
    onSelect(value === "" ? null : value);
  }

  function commitSave() {
    const name = newName.trim();
    if (!name) return;
    onSave(name);
    newName = "";
    naming = false;
  }
</script>

<div class="flex items-center gap-2">
  <Icon name="layers" size={18} class="shrink-0 text-muted" />

  <div class="relative">
    <select
      value={config.active_profile ?? ""}
      onchange={handleSelect}
      class="min-w-[180px] cursor-pointer appearance-none rounded-lg border border-line-strong
        bg-surface-2 py-2 pl-3 pr-9 text-sm text-fg transition-colors hover:bg-surface-3
        focus:border-brand focus:outline-none focus:ring-2 focus:ring-brand/30"
    >
      <option value="">Kein Profil (manuell)</option>
      {#each config.profiles as p (p.id)}
        <option value={p.id}>{p.name}</option>
      {/each}
    </select>
    <Icon
      name="expand_more"
      size={18}
      class="pointer-events-none absolute right-2.5 top-1/2 -translate-y-1/2 text-muted"
    />
  </div>

  {#if naming}
    <input
      class="field-input w-44"
      placeholder="Profilname…"
      bind:value={newName}
      onkeydown={(e) => e.key === "Enter" && commitSave()}
    />
    <button class="btn btn-primary" onclick={commitSave} aria-label="Speichern">
      <Icon name="check" size={18} />
    </button>
    <button
      class="btn-icon"
      onclick={() => (naming = false)}
      aria-label="Abbrechen"
    >
      <Icon name="close" size={18} />
    </button>
  {:else}
    <button
      class="btn btn-ghost"
      title="Aktuelle Toggle-Kombination als Profil speichern"
      onclick={() => {
        naming = true;
        newName = "";
      }}
    >
      <Icon name="bookmark_add" size={18} />
      Profil
    </button>
    {#if config.active_profile}
      <button
        class="btn btn-danger"
        title="Aktives Profil löschen"
        onclick={() => onDelete(config.active_profile!)}
      >
        <Icon name="delete" size={18} />
      </button>
    {/if}
  {/if}
</div>
