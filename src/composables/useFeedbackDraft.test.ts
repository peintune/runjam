import { describe, it, expect, vi } from 'vitest';
import { createFeedbackDraft } from './useFeedbackDraft';

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
