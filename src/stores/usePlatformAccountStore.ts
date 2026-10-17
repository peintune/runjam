import {defineStore} from 'pinia';
import {computed,ref} from 'vue';
import {platformAccount,type PlatformAccount,type PlatformCatalog,type LedgerItem} from '../api/platform-account';
import {t} from '../i18n';
export const usePlatformAccountStore=defineStore('platform-account',()=>{
 const account=ref<PlatformAccount|null>(null),catalog=ref<PlatformCatalog|null>(null),items=ref<LedgerItem[]>([]);
 const loading=ref(false),busy=ref(false),fresh=ref(false),error=ref('');
 const pending=ref<{orderId:string;createdAt:number}|null>(null),paymentState=ref<'idle'|'pending'|'paid'|'expired'|'refunded'>('idle');
 let epoch=0,checking=false;
 const loggedIn=computed(()=>!!account.value),canRecharge=computed(()=>!!account.value&&fresh.value&&catalog.value?.paymentAvailable&&!!catalog.value.products.ai10);
 const message=(e:unknown)=>typeof e==='string'?e:e instanceof Error?e.message:t('account.serviceError');
 async function ledger(version:number){
  if(!account.value)return;
  const result=await platformAccount.ledger();if(epoch===version)items.value=result.items||[];
 }
 async function refresh(){
  if(loading.value||busy.value)return;
  const version=epoch;loading.value=true;
  try{const result=await platformAccount.status();if(epoch!==version)return;
   account.value=result.account;catalog.value=result.catalog;fresh.value=true;error.value='';
   if(!result.account){items.value=[];pending.value=null;}else await ledger(version);
  }catch(e){if(epoch===version){fresh.value=false;error.value=message(e);}}
  finally{if(epoch===version)loading.value=false;}
 }
 async function sendCode(email:string){return platformAccount.sendCode(email.trim());}
 async function login(email:string,code:string){
  if(busy.value)return;const version=++epoch;busy.value=true;error.value='';loading.value=false;
  try{const result=await platformAccount.login(email.trim(),code.trim());if(epoch!==version)return;
   account.value=result.account;catalog.value=result.catalog;fresh.value=true;pending.value=null;paymentState.value='idle';await ledger(version);
  }catch(e){if(epoch===version)error.value=message(e);}
  finally{if(epoch===version)busy.value=false;}
 }
 async function logout(){
  ++epoch;loading.value=false;busy.value=true;error.value='';
  try{await platformAccount.logout();account.value=null;items.value=[];pending.value=null;paymentState.value='idle';fresh.value=false;}
  catch(e){error.value=message(e);}finally{busy.value=false;}
 }
 async function recharge(){
  if(!canRecharge.value){error.value=t(account.value?'account.serviceError':'account.loginRequired');return;}
  if(busy.value)return;const version=epoch;busy.value=true;error.value='';
  try{const result=await platformAccount.checkout();if(epoch!==version)return;
   pending.value={orderId:result.orderId,createdAt:pending.value?.orderId===result.orderId?pending.value.createdAt:Date.now()};paymentState.value='pending';
  }catch(e){if(epoch===version)error.value=message(e);}finally{if(epoch===version)busy.value=false;}
 }
 async function checkPayment(){
  if(!pending.value||!account.value||checking)return;
  const version=epoch,order=pending.value;
  if(Date.now()-order.createdAt>300000){pending.value=null;paymentState.value='expired';return;}
  checking=true;
  try{const result=await platformAccount.order(order.orderId);if(epoch!==version||pending.value?.orderId!==order.orderId)return;
   if(result.status==='paid'||result.status==='refunded'){
    // Keep polling until the refresh after confirmation actually completes.
    // A pre-payment refresh may still be in flight and return an old snapshot.
    if(loading.value||busy.value)return;
    await refresh();
    if(epoch===version&&pending.value?.orderId===order.orderId&&fresh.value){paymentState.value=result.status;pending.value=null;}
   }
  }catch(e){if(epoch===version)error.value=message(e);}finally{checking=false;}
 }
 return {account,catalog,items,loading,busy,fresh,error,pending,paymentState,loggedIn,canRecharge,refresh,sendCode,login,logout,recharge,checkPayment};
});
