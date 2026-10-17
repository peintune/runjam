import {beforeEach,describe,it,expect,vi} from 'vitest';
import {createPinia,setActivePinia} from 'pinia';
const api=vi.hoisted(()=>({status:vi.fn(),sendCode:vi.fn(),login:vi.fn(),logout:vi.fn(),ledger:vi.fn(),checkout:vi.fn(),order:vi.fn()}));
vi.mock('../api/platform-account',()=>({platformAccount:api}));
import {usePlatformAccountStore} from './usePlatformAccountStore';
const snapshot={catalog:{paymentAvailable:true,hosted:{available:true},products:{ai10:{amountMicros:10000000}}},account:{user:{id:'u',email:'test@example.com'},wallet:{balanceMicros:10000000,availableMicros:10000000,rechargedMicros:10000000,spentMicros:0,reservedMicros:0}}};
beforeEach(()=>{setActivePinia(createPinia());vi.resetAllMocks();api.ledger.mockResolvedValue({items:[]});});
describe('RunJam native account lifecycle',()=>{
 it('only server-confirmed payment refreshes balance; opening checkout remains pending',async()=>{
  api.status.mockResolvedValue(snapshot);api.checkout.mockResolvedValue({orderId:'order'});api.order.mockResolvedValue({orderId:'order',status:'pending'});
  const store=usePlatformAccountStore();await store.refresh();await store.recharge();expect(store.pending?.orderId).toBe('order');expect(store.paymentState).toBe('pending');
  await store.checkPayment();expect(store.paymentState).toBe('pending');
  api.order.mockResolvedValue({orderId:'order',status:'paid'});await store.checkPayment();expect(store.paymentState).toBe('paid');expect(store.pending).toBeNull();expect(api.status).toHaveBeenCalledTimes(2);
 });
 it('rejects top-up without login and surfaces connectivity errors',async()=>{
  const store=usePlatformAccountStore();await store.recharge();expect(api.checkout).not.toHaveBeenCalled();expect(store.error).toBeTruthy();
  api.status.mockRejectedValue('network unavailable');await store.refresh();expect(store.error).toContain('network unavailable');expect(store.account).toBeNull();
 });
 it('late refresh responses cannot restore an account after logout',async()=>{
  let resolve!:(v:typeof snapshot)=>void;api.status.mockReturnValue(new Promise(r=>resolve=r));api.logout.mockResolvedValue({ok:true});
  const store=usePlatformAccountStore();const refresh=store.refresh();await store.logout();resolve(snapshot);await refresh;expect(store.account).toBeNull();
 });
 it('never automatically charges or switches models when logging in',async()=>{
  api.login.mockResolvedValue(snapshot);const store=usePlatformAccountStore();await store.login('test@example.com','12345678');expect(store.account?.user.email).toBe('test@example.com');expect(api.checkout).not.toHaveBeenCalled();
 });
});
it('payment confirmation keeps polling until a post-payment refresh actually succeeds',async()=>{
 api.status.mockResolvedValue(snapshot);api.checkout.mockResolvedValue({orderId:'order'});api.order.mockResolvedValue({orderId:'order',status:'paid'});
 const store=usePlatformAccountStore();await store.refresh();await store.recharge();
 let resolve!:(v:typeof snapshot)=>void;api.status.mockReturnValueOnce(new Promise(r=>resolve=r));
 const oldRefresh=store.refresh();await store.checkPayment();expect(store.paymentState).toBe('pending');expect(store.pending).not.toBeNull();
 resolve({...snapshot,account:{...snapshot.account,wallet:{...snapshot.account.wallet,availableMicros:0}}});await oldRefresh;
 await store.checkPayment();expect(store.paymentState).toBe('paid');expect(store.account?.wallet.availableMicros).toBe(10000000);
});
