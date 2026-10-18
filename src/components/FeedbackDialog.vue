<script setup lang="ts">
import { watch } from 'vue';
import { Check, MessageCircle, X } from 'lucide-vue-next';
import { useFeedbackDraft } from '../composables/useFeedbackDraft';
import type { FeedbackType } from '../api/telemetry';
import type { TranslationKey } from '../i18n';

const props = defineProps<{ open: boolean }>();
const emit = defineEmits<{ close: [] }>();
const { type, content, email, sending, done, error, submit } = useFeedbackDraft();
const types: { id: FeedbackType; label: TranslationKey }[] = [
  { id: 'bug', label: 'session.feedbackBug' },
  { id: 'feature', label: 'session.feedbackFeature' },
  { id: 'other', label: 'session.feedbackOther' },
];
watch(() => props.open, (open) => { if (open) done.value = false; });
function close() { if (!sending.value) emit('close'); }
</script>

<template>
  <Teleport to="body">
    <div v-if="open" class="fixed inset-0 z-[100] flex items-center justify-center">
      <div class="absolute inset-0 bg-black/30" @click="close" />
      <section role="dialog" aria-modal="true" aria-labelledby="feedback-title" class="relative w-[420px] max-w-[90vw] bg-white rounded-2xl shadow-2xl p-5" @keydown.esc="close">
        <div class="flex items-center justify-between mb-3">
          <h2 id="feedback-title" class="flex items-center gap-2 text-[15px] font-semibold text-gray-900"><MessageCircle :size="16" />{{ $t('session.feedbackTitle') }}</h2>
          <button :aria-label="$t('common.close')" :disabled="sending" class="p-1 text-gray-400 hover:bg-gray-100 rounded" @click="close"><X :size="16" /></button>
        </div>
        <div v-if="done" role="status" class="py-8 flex flex-col items-center gap-2">
          <Check :size="24" class="text-green-600" />
          <p>{{ $t('session.feedbackThanks') }}</p>
          <p class="text-[12px] text-gray-500">{{ $t('session.feedbackThanksDesc') }}</p>
          <button class="mt-3 rounded-lg px-4 py-2 bg-indigo-600 text-white" @click="close">{{ $t('common.close') }}</button>
        </div>
        <form v-else @submit.prevent="submit">
          <div class="flex gap-1.5 mb-3">
            <button v-for="item in types" :key="item.id" type="button" :disabled="sending" :aria-pressed="type === item.id" class="flex-1 px-3 py-1.5 rounded-lg text-[12px] border" :class="type === item.id ? 'bg-gray-900 text-white border-gray-900' : 'border-gray-200 text-gray-600'" @click="type = item.id">{{ $t(item.label) }}</button>
          </div>
          <label for="feedback-content" class="sr-only">{{ $t('session.feedbackPlaceholder') }}</label>
          <textarea id="feedback-content" v-model="content" :disabled="sending" maxlength="10000" required rows="5" :placeholder="$t('session.feedbackPlaceholder')" class="w-full px-3 py-2.5 rounded-xl border border-gray-200 text-[13px] text-gray-900 bg-white resize-none" />
          <p class="text-right text-[11px] text-gray-400">{{ content.length }}/10000</p>
          <label for="feedback-email" class="sr-only">{{ $t('session.feedbackEmailPlaceholder') }}</label>
          <input id="feedback-email" v-model="email" :disabled="sending" type="email" maxlength="254" :placeholder="$t('session.feedbackEmailPlaceholder')" class="mt-2 w-full h-9 px-3 rounded-xl border border-gray-200 text-[13px] text-gray-900 bg-white" />
          <p class="mt-2 text-[11px] text-gray-500">{{ $t('session.feedbackPrivacy') }}</p>
          <p v-if="error" role="alert" class="mt-2 text-[12px] text-red-600">{{ error }}</p>
          <div class="mt-4 flex justify-end gap-2">
            <button type="button" :disabled="sending" class="px-4 py-2 rounded-xl text-[13px] text-gray-600 hover:bg-gray-100" @click="close">{{ $t('common.cancel') }}</button>
            <button type="submit" :disabled="!content.trim() || content.length > 10000 || sending" class="px-4 py-2 rounded-xl text-[13px] text-white bg-indigo-600 disabled:bg-gray-300">{{ sending ? $t('session.feedbackSending') : $t('session.feedbackSubmit') }}</button>
          </div>
        </form>
      </section>
    </div>
  </Teleport>
</template>
