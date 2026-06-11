<script lang="ts">
  let {
    name,
    size = 18,
    fill = false,
    class: cls = "",
  }: {
    name: string;
    size?: number;
    fill?: boolean;
    class?: string;
  } = $props();
</script>

<span
  class="material-symbols-rounded leading-none {fill ? 'icon-fill' : ''} {cls}"
  style="font-size: {size}px;"
  aria-hidden="true">{name}</span>
