<script setup lang="ts">
// 纯展示组件：把消息的 markdown HTML 渲染为内容。
//
// 为什么拆出来：父组件 ChatMessages 每次重渲染（打字机 tick / now tick）
// 都会重建整份消息列表的 vnode，Vue 对 v-html 的 patch 不做字符串相等性
// 检查，会无条件重设 innerHTML——即使内容一字未变，浏览器也要重新解析整段
// HTML（长消息含 hljs 高亮代码块时尤其昂贵）。抽成独立组件后，html prop
// 不变时 Vue 直接跳过本组件的更新，innerHTML 完全不动；只有真正变化的消息
// 才会重设 DOM。data-msg-content 与 @click 由父组件 fallthrough 到根节点。
defineProps<{ html: string }>();
</script>

<template>
  <div
    class="md-content prose prose-base max-w-none
      prose-p:text-[15px] prose-p:leading-[1.75] prose-p:text-[#1e1e2e] prose-p:my-2.5
      prose-headings:text-[#111127] prose-headings:font-semibold prose-headings:tracking-tight
      prose-h1:text-[22px] prose-h1:mt-6 prose-h1:mb-3
      prose-h2:text-[18px] prose-h2:mt-5 prose-h2:mb-2.5
      prose-h3:text-[15px] prose-h3:mt-4 prose-h3:mb-2
      prose-blockquote:border-l-[3px] prose-blockquote:border-indigo-200 prose-blockquote:pl-4 prose-blockquote:my-4 prose-blockquote:text-[#4a4a6a] prose-blockquote:not-italic prose-blockquote:text-[14px]
      prose-code:bg-[#f1f4f9] prose-code:text-[#c14a6b] prose-code:px-[5px] prose-code:py-[2px] prose-code:rounded-[4px] prose-code:text-[13px] prose-code:font-medium prose-code:before:content-none prose-code:after:content-none
      prose-pre:bg-transparent prose-pre:p-0 prose-pre:m-0 prose-pre:rounded-none
      prose-a:text-indigo-500 prose-a:no-underline hover:prose-a:underline prose-a:font-medium
      prose-strong:text-[#111127] prose-strong:font-semibold
      prose-ul:my-3 prose-ol:my-3 prose-li:my-1 prose-li:leading-[1.75] prose-li:text-[15px] prose-li:text-[#1e1e2e]
      prose-table:text-[13px] prose-th:border prose-th:border-[#e4e7ed] prose-th:bg-[#f8f9fc] prose-th:px-3 prose-th:py-2 prose-th:font-semibold prose-th:text-[#111127] prose-td:border prose-td:border-[#e4e7ed] prose-td:px-3 prose-td:py-2
      prose-hr:my-5 prose-hr:border-[#e4e7ed]
      prose-img:rounded-xl"
    v-html="html"
  />
</template>
