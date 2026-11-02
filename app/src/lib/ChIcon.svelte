<script lang="ts">
  import { CH, type Channel } from "./api";
  import Logo from "./Logo.svelte";
  let { ch, size = "" }: { ch: Channel; size?: string } = $props();
</script>

{#if ch === "all"}
  <span class="ic {size} all"><Logo /></span>
{:else}
  <span class="ic {size}" style="background-image:url({CH[ch].logo})"></span>
{/if}

<style>
  /* 尺寸写在组件里而不是全局样式：图标是它自己的事。
     放进全局最容易在改版时被漏掉，而漏掉的后果是宽高归零——图标直接消失。 */
  .ic {
    width: 28px;
    height: 28px;
    flex: none;
    border-radius: 8px;
    background-size: 100% 100%;
    background-position: center;
    background-repeat: no-repeat;
    box-shadow: inset 0 0 0 1px rgba(24, 22, 16, 0.07);
  }
  .ic.xs {
    width: 17px;
    height: 17px;
    border-radius: 5px;
  }
  /* 胶囊里用：46px 高的条，图标不能按 28 走 */
  .ic.sm {
    width: 22px;
    height: 22px;
    border-radius: 6px;
  }
  .all {
    background: #171a2e;
    display: grid;
    place-items: center;
  }
  .all :global(svg) {
    width: 78%;
    height: 78%;
  }
</style>
