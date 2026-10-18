import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { createFeedbackDraft } from './useFeedbackDraft';

let stored: Map<string, string>;
const draftKey = 'runjam.feedbackDraft.v1';
beforeEach(() => {
  stored = new Map();
  vi.stubGlobal('localStorage', {
    getItem: (key: string) => stored.get(key) ?? null,
    setItem: (key: string, value: string) => { stored.set(key, value); },
    removeItem: (key: string) => { stored.delete(key); },
  });
});
afterEach(() => { vi.unstubAllGlobals(); });

describe('feedback draft delivery', () => {
  it('keeps content and request identity after a failed acknowledgement, then clears only after confirmed persistence', async () => {
    const sent: string[] = [];
    let attempt = 0;
    const draft = createFeedbackDraft(async (_type, _content, _email, requestId) => {
      sent.push(requestId);
      if (++attempt === 1) throw new Error('offline');
      return { ok: true, id: '2337ab27-69db-42c9-bf68-243017e28b5e' };
    });
    draft.content.value = 'A useful suggestion';
    await draft.submit();
    expect(draft.error.value).toBe('Error: offline');
    expect(draft.content.value).toBe('A useful suggestion');
    expect(draft.done.value).toBe(false);
    await draft.submit();
    expect(sent[0]).toMatch(/^[a-f0-9-]{36}$/);
    expect(sent[1]).toBe(sent[0]);
    expect(draft.done.value).toBe(true);
    expect(draft.content.value).toBe('');
  });

  it('does not treat malformed acknowledgement as success and gives an edited retry a fresh identity', async () => {
    const ids: string[] = [];
    const draft = createFeedbackDraft(async (_type, _content, _email, id) => {
      ids.push(id);
      return { ok: false, id: '' } as never;
    });
    draft.content.value = 'Before';
    await draft.submit();
    expect(draft.content.value).toBe('Before');
    expect(draft.done.value).toBe(false);
    expect(draft.error.value).not.toBe('');
    draft.content.value = 'After';
    await draft.submit();
    expect(ids[1]).not.toBe(ids[0]);
  });

  it('retains the draft when a success response lacks a persisted UUID', async () => {
    const draft = createFeedbackDraft(async () => ({ok: true, id: 'queued'}));
    draft.content.value = 'Do not lose this draft';
    await draft.submit();
    expect(draft.done.value).toBe(false);
    expect(draft.content.value).toBe('Do not lose this draft');
    expect(draft.error.value).not.toBe('');
  });

  it('creates retry IDs on WebViews that provide getRandomValues without randomUUID', async () => {
    const nativeCrypto = globalThis.crypto;
    vi.stubGlobal('crypto', { getRandomValues: nativeCrypto.getRandomValues.bind(nativeCrypto) });
    try {
      let sentId = '';
      const draft = createFeedbackDraft(async (_type, _content, _email, id) => {
        sentId = id;
        return {ok: true, id: '2337ab27-69db-42c9-bf68-243017e28b5e'};
      });
      draft.content.value = 'Compatible WebView';
      await draft.submit();
      expect(draft.done.value).toBe(true);
      expect(sentId).toMatch(/^[a-f0-9]{8}-[a-f0-9]{4}-4[a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$/);
    } finally { vi.unstubAllGlobals(); }
  });

  it('rejects oversized content and suppresses duplicate submits while busy', async () => {
    let calls = 0;
    let finish!: (v: {ok: true; id: string}) => void;
    const draft = createFeedbackDraft(() => { calls++; return new Promise(resolve => { finish = resolve; }); });
    draft.content.value = 'a'.repeat(10001);
    await draft.submit();
    expect(calls).toBe(0);
    draft.content.value = 'Valid';
    const pending = draft.submit();
    await draft.submit();
    expect(calls).toBe(1);
    expect(draft.sending.value).toBe(true);
    finish({ok:true, id:'2337ab27-69db-42c9-bf68-243017e28b5e'});
    await pending;
    expect(draft.sending.value).toBe(false);
  });
});

describe('feedback draft persistence', () => {
  it('restores unsent fields immediately after closing or restarting without persisting runtime state', () => {
    const draft = createFeedbackDraft();
    draft.type.value = 'feature';
    draft.content.value = 'Keep this unsent suggestion';
    draft.email.value = 'reply@example.com';
    const reopened = createFeedbackDraft();
    expect(reopened.type.value).toBe('feature');
    expect(reopened.content.value).toBe('Keep this unsent suggestion');
    expect(reopened.email.value).toBe('reply@example.com');
    expect(reopened.sending.value).toBe(false);
    expect(reopened.done.value).toBe(false);
    expect(reopened.error.value).toBe('');
    expect(Object.keys(JSON.parse(stored.get(draftKey)!)).sort()).toEqual(['content', 'email', 'requestId', 'type', 'version']);
  });

  it('persists the request ID before sending and reuses it after an offline restart, clearing only on UUID acknowledgement', async () => {
    let firstId = '';
    let savedIdAtSend: unknown;
    const draft = createFeedbackDraft(async (_type, _content, _email, id) => {
      firstId = id;
      const saved = stored.get(draftKey);
      savedIdAtSend = saved ? JSON.parse(saved).requestId : undefined;
      throw new Error('offline');
    });
    draft.content.value = 'Failed feedback';
    draft.email.value = 'reply@example.com';
    await draft.submit();
    let retryId = '';
    const restarted = createFeedbackDraft(async (_type, text, email, id) => {
      retryId = id;
      expect(text).toBe('Failed feedback');
      expect(email).toBe('reply@example.com');
      return {ok: true, id: '2337ab27-69db-42c9-bf68-243017e28b5e'};
    });
    expect(restarted.content.value).toBe('Failed feedback');
    await restarted.submit();
    expect(savedIdAtSend).toBe(firstId);
    expect(retryId).toBe(firstId);
    expect(restarted.done.value).toBe(true);
    expect(stored.has(draftKey)).toBe(false);
    expect(createFeedbackDraft().content.value).toBe('');
  });

  it('invalidates the old request ID when content is edited and survives another restart with the replacement ID', async () => {
    const ids: string[] = [];
    const fail = async (_type: string, _text: string, _email: string | undefined, id: string): Promise<never> => {
      ids.push(id);
      throw new Error('offline');
    };
    const draft = createFeedbackDraft(fail);
    draft.content.value = 'Before';
    await draft.submit();
    draft.content.value = 'After';
    const restarted = createFeedbackDraft(fail);
    expect(restarted.content.value).toBe('After');
    await restarted.submit();
    await createFeedbackDraft(fail).submit();
    expect(ids).toHaveLength(3);
    expect(ids[1]).not.toBe(ids[0]);
    expect(ids[2]).toBe(ids[1]);
  });

  it('keeps malformed acknowledgements retryable across restarts', async () => {
    const draft = createFeedbackDraft(async () => ({ok: true, id: 'queued'}));
    draft.content.value = 'Unacknowledged';
    await draft.submit();
    expect(createFeedbackDraft().content.value).toBe('Unacknowledged');
    expect(JSON.parse(stored.get(draftKey)!).requestId).toMatch(/^[a-f0-9-]{36}$/);
  });

  it.each(['{broken json', JSON.stringify({version:1,type:'feature',content:45,email:'',requestId:''}),
    JSON.stringify({version:1,type:'feature',content:'a'.repeat(10001),email:'',requestId:''})])('ignores corrupt local storage without crashing startup (%#)', value => {
    stored.set(draftKey, value);
    const draft = createFeedbackDraft();
    expect(draft.content.value).toBe('');
    expect(draft.email.value).toBe('');
  });

  it('recovers valid draft text but never retries an invalid stored request ID', async () => {
    stored.set(draftKey, JSON.stringify({version:1,type:'other',content:'Recoverable',email:'',requestId:'invalid'}));
    let sentId = '';
    const draft = createFeedbackDraft(async (_type, _content, _email, id) => {
      sentId = id;
      return {ok:true, id:'2337ab27-69db-42c9-bf68-243017e28b5e'};
    });
    expect(draft.content.value).toBe('Recoverable');
    await draft.submit();
    expect(sentId).toMatch(/^[a-f0-9]{8}-[a-f0-9]{4}-4[a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$/);
  });

  it('does not clear a newer locally edited draft when an earlier submission is acknowledged', async () => {
    let acknowledge!: (value: {ok: true; id: string}) => void;
    const draft = createFeedbackDraft(() => new Promise(resolve => { acknowledge = resolve; }));
    draft.content.value = 'Original submission';
    const pending = draft.submit();
    draft.content.value = 'Newer draft';
    acknowledge({ok:true, id:'2337ab27-69db-42c9-bf68-243017e28b5e'});
    await pending;
    expect(createFeedbackDraft().content.value).toBe('Newer draft');
    expect(draft.done.value).toBe(false);
    expect(JSON.parse(stored.get(draftKey)!).requestId).toBe('');
  });

  it('continues safely in memory when storage cannot be read, written or cleared', async () => {
    vi.stubGlobal('localStorage', {
      getItem: () => { throw new Error('denied'); },
      setItem: () => { throw new Error('quota'); },
      removeItem: () => { throw new Error('denied'); },
    });
    const draft = createFeedbackDraft(async () => ({ok:true, id:'2337ab27-69db-42c9-bf68-243017e28b5e'}));
    draft.content.value = 'In memory fallback';
    await draft.submit();
    expect(draft.done.value).toBe(true);
    expect(draft.content.value).toBe('');
  });
});
