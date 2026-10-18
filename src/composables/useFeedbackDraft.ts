import { ref, watch } from 'vue';
import { submitFeedback, type FeedbackAck, type FeedbackType } from '../api/telemetry';

const DRAFT_KEY = 'runjam.feedbackDraft.v1';
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
interface SavedDraft {
  version: 1;
  type: FeedbackType;
  content: string;
  email: string;
  requestId: string;
}

function loadDraft(): SavedDraft | undefined {
  try {
    const raw = localStorage.getItem(DRAFT_KEY);
    if (!raw) return;
    const value = JSON.parse(raw);
    if (!value || value.version !== 1 || !['bug', 'feature', 'other'].includes(value.type)
      || typeof value.content !== 'string' || value.content.length > 10000
      || typeof value.email !== 'string' || value.email.length > 254) {
      localStorage.removeItem(DRAFT_KEY);
      return;
    }
    return {
      version: 1, type: value.type, content: value.content, email: value.email,
      requestId: typeof value.requestId === 'string' && UUID.test(value.requestId) ? value.requestId : '',
    };
  } catch {
    // Storage may be denied or contain an incomplete/corrupt write.
    return;
  }
}

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
  const restored = loadDraft();
  const type = ref<FeedbackType>(restored?.type ?? 'bug');
  const content = ref(restored?.content ?? '');
  const email = ref(restored?.email ?? '');
  const sending = ref(false);
  const done = ref(false);
  const error = ref('');
  const currentBody = () => JSON.stringify({ type: type.value, content: content.value.trim(), email: email.value.trim() || undefined });
  let requestId = restored?.requestId ?? '';
  let requestBody = requestId ? currentBody() : '';
  let clearing = false;
  function persist() {
    try {
      if (!content.value && !email.value) {
        localStorage.removeItem(DRAFT_KEY);
      } else {
        // Deliberately persist only user-entered fields and retry identity.
        const saved: SavedDraft = { version: 1, type: type.value, content: content.value, email: email.value, requestId };
        localStorage.setItem(DRAFT_KEY, JSON.stringify(saved));
      }
    } catch {
      // Continue with the in-memory draft when browser storage is unavailable.
    }
  }
  watch([type, content, email], () => {
    if (clearing) return;
    if (currentBody() !== requestBody) {
      requestId = '';
      requestBody = '';
    }
    // Synchronous persistence also covers closing immediately after an edit.
    persist();
  }, { flush: 'sync' });
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
    // Save the identity before sending: an acknowledgement can be lost offline.
    persist();
    sending.value = true;
    error.value = '';
    done.value = false;
    try {
      const ack = await send(type.value, text, address, requestId);
      if (ack?.ok !== true || typeof ack.id !== 'string' || !UUID.test(ack.id)) {
        throw new Error('Feedback was not acknowledged. Please retry.');
      }
      if (currentBody() === body) {
        clearing = true;
        done.value = true;
        content.value = '';
        email.value = '';
        requestId = '';
        requestBody = '';
        persist();
        clearing = false;
      }
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
