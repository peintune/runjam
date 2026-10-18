import { ref } from 'vue';
import { submitFeedback, type FeedbackAck, type FeedbackType } from '../api/telemetry';

// getRandomValues also works on supported older WebViews without randomUUID.
function newRequestId(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const hex = Array.from(bytes, b => b.toString(16).padStart(2, '0')).join('');
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

type Sender = (type: FeedbackType, content: string, email: string | undefined, requestId: string) => Promise<FeedbackAck>;

export function createFeedbackDraft(send: Sender = submitFeedback) {
  const type = ref<FeedbackType>('bug');
  const content = ref('');
  const email = ref('');
  const sending = ref(false);
  const done = ref(false);
  const error = ref('');
  let requestId = '';
  let requestBody = '';
  async function submit() {
    if (sending.value) return;
    const text = content.value.trim();
    if (!text || content.value.length > 10000) {
      error.value = 'Feedback must contain between 1 and 10000 characters.';
      return;
    }
    const address = email.value.trim() || undefined;
    const body = JSON.stringify({ type: type.value, content: text, email: address });
    if (!requestId || body !== requestBody) {
      requestId = newRequestId();
      requestBody = body;
    }
    sending.value = true;
    error.value = '';
    done.value = false;
    try {
      const ack = await send(type.value, text, address, requestId);
      if (ack?.ok !== true || typeof ack.id !== 'string' || !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(ack.id)) {
        throw new Error('Feedback was not acknowledged. Please retry.');
      }
      done.value = true;
      content.value = '';
      email.value = '';
      requestId = '';
      requestBody = '';
    } catch (e) {
      error.value = String(e);
    } finally {
      sending.value = false;
    }
  }
  return { type, content, email, sending, done, error, submit };
}

const draft = createFeedbackDraft();
export function useFeedbackDraft() { return draft; }
