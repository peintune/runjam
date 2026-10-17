<script setup lang="ts">
import {computed,ref,onMounted,onUnmounted} from 'vue';
import {Mail,ArrowUpRight,RefreshCw,LogOut,Wallet,Check,LoaderCircle,Clock3} from 'lucide-vue-next';
import {usePlatformAccountStore} from '../../stores/usePlatformAccountStore';
import {t,currentLocale} from '../../i18n';
const store=usePlatformAccountStore(),email=ref(''),code=ref(''),formError=ref(''),sent=ref(false),sending=ref(false),cooldown=ref(0);
const codeInput=ref<HTMLInputElement|null>(null),filter=ref('all');
let poll:ReturnType<typeof setInterval>|undefined,countdown:ReturnType<typeof setInterval>|undefined;
const rows=computed(()=>filter.value==='all'?store.items:store.items.filter(x=>x.kind===filter.value));
const money=(v:number,digits=4)=>new Intl.NumberFormat(currentLocale(),{minimumFractionDigits:2,maximumFractionDigits:digits}).format(v/1000000);
const when=(v:number)=>new Intl.DateTimeFormat(currentLocale(),{month:'2-digit',day:'2-digit',hour:'2-digit',minute:'2-digit'}).format(v);
const label=(kind:string)=>t(kind==='recharge'?'account.rechargeLabel':kind==='refund'?'account.refundLabel':'account.usageLabel');
const paymentText=computed(()=>store.paymentState==='pending'?t('account.pending'):store.paymentState==='paid'?t('account.paid'):store.paymentState==='expired'?t('account.expired'):store.paymentState==='refunded'?t('account.refunded'):'');
async function sendCode(){
 if(sending.value||cooldown.value)return;
 if(!/^\S+@[^\s@]+\.[^\s@]+$/.test(email.value.trim())){formError.value=t('account.invalidEmail');return;}
 sending.value=true;formError.value='';
 try{await store.sendCode(email.value);sent.value=true;cooldown.value=60;codeInput.value?.focus();}
 catch(e){formError.value=String(e);}finally{sending.value=false;}
}
async function login(){formError.value='';await store.login(email.value,code.value);if(store.loggedIn)code.value='';}
async function focusRefresh(){await store.refresh();await store.checkPayment();}
onMounted(()=>{
 void store.refresh();window.addEventListener('focus',focusRefresh);
 poll=setInterval(()=>{if(document.visibilityState==='visible'&&store.pending)void store.checkPayment();},3000);
 countdown=setInterval(()=>{if(cooldown.value>0)cooldown.value--;},1000);
});
onUnmounted(()=>{clearInterval(poll);clearInterval(countdown);window.removeEventListener('focus',focusRefresh);});
</script>

<template>
 <div class="account-page">
  <header class="page-heading"><div><h1>{{ $t('account.title') }}</h1><p>{{ $t('account.subtitle') }}</p></div>
   <button class="icon-button" :title="$t('account.refresh')" :aria-label="$t('account.refresh')" :disabled="store.loading||store.busy" @click="focusRefresh"><RefreshCw :size="17" :class="{'animate-spin':store.loading}"/></button>
  </header>
  <div v-if="store.error" role="alert" class="error-message">{{ store.error }}</div>
  <section class="wallet-panel">
   <div class="wallet-summary">
    <div class="wallet-eyebrow"><Wallet :size="17"/><span>{{ $t('account.wallet') }}</span></div>
    <div class="balance"><span class="currency">¥</span>{{ store.account?money(store.account.wallet.availableMicros):'—' }}</div>
    <p class="balance-caption">{{ $t('account.available') }}</p>
    <div v-if="store.account" class="account-identity"><Mail :size="14"/><span>{{ store.account.user.email }}</span><button @click="store.logout" :disabled="store.busy" :title="$t('account.logout')"><LogOut :size="14"/><span>{{ $t('account.logout') }}</span></button></div>
    <p v-else class="signed-out-hint">{{ $t('account.loginHistory') }}</p>
   </div>
   <div v-if="!store.account" class="login-panel">
    <h2>{{ $t('account.loginTitle') }}</h2>
    <form @submit.prevent="login">
     <label for="platform-email">{{ $t('account.email') }}</label>
     <input id="platform-email" v-model="email" type="email" autocomplete="email" required :placeholder="$t('account.emailPlaceholder')" :disabled="store.busy" />
     <label for="platform-code">{{ $t('account.code') }}</label>
     <div class="code-row"><input id="platform-code" ref="codeInput" v-model="code" inputmode="numeric" autocomplete="one-time-code" pattern="[0-9]{6,10}" minlength="6" maxlength="10" required :disabled="store.busy" placeholder="000000"/>
      <button type="button" class="secondary-button" @click="sendCode" :disabled="sending||cooldown>0||store.busy">{{ cooldown?$t('account.sendAgain',{seconds:cooldown}):$t('account.sendCode') }}</button>
     </div>
     <p v-if="formError" role="alert" class="form-error">{{ formError }}</p><p v-else-if="sent" role="status" class="form-note">{{ $t('account.codeSent') }}</p>
     <button type="submit" class="primary-button login-button" :disabled="store.busy||!email.trim()||!code.trim()"><LoaderCircle v-if="store.busy" :size="15" class="animate-spin"/>{{ store.busy?$t('account.processing'):$t('account.login') }}</button>
    </form>
   </div>
   <div v-else class="topup-panel">
    <div class="topup-value"><span>¥</span>{{ money(store.catalog?.products.ai10?.amountMicros||10000000,2) }}</div>
    <button class="primary-button" @click="store.recharge" :disabled="!store.canRecharge||store.busy"><LoaderCircle v-if="store.busy" :size="16" class="animate-spin"/><span>{{ store.busy?$t('account.processing'):store.pending?$t('account.reopen'):$t('account.topup') }}</span><ArrowUpRight v-if="!store.busy" :size="16"/></button>
    <p>{{ store.catalog?.paymentAvailable?$t('account.topupHint'):$t('account.topupUnavailable') }}</p>
   </div>
  </section>
  <div v-if="paymentText" role="status" :class="['payment-note',{'paid':store.paymentState==='paid'}]"><Check v-if="store.paymentState==='paid'" :size="16"/><Clock3 v-else :size="16"/>{{ paymentText }}</div>
  <div class="wallet-footnote">{{ $t('account.freeHint') }}</div>
  <div v-if="store.account" class="wallet-stats">
   <div><span>{{ $t('account.recharged') }}</span><strong>¥{{ money(store.account.wallet.rechargedMicros,2) }}</strong></div>
   <div><span>{{ $t('account.spent') }}</span><strong>¥{{ money(store.account.wallet.spentMicros,6) }}</strong></div>
   <div><span>{{ $t('account.reserved') }}</span><strong>¥{{ money(store.account.wallet.reservedMicros,6) }}</strong></div>
  </div>
  <section class="history-section">
   <div class="history-heading"><h2>{{ $t('account.history') }}</h2><div v-if="store.account" class="history-filter"><button v-for="kind in ['all','recharge','usage','refund']" :key="kind" @click="filter=kind" :class="{active:filter===kind}">{{ kind==='all'?$t('account.all'):label(kind) }}</button></div></div>
   <div v-if="!store.account||rows.length===0" class="history-empty"><Wallet :size="24"/><p>{{ store.account?$t('account.emptyHistory'):$t('account.loginHistory') }}</p></div>
   <div v-else class="history-list"><div v-for="(item,index) in rows" :key="item.createdAt+'-'+index" class="transaction"><span :class="['transaction-icon',item.kind]"><ArrowUpRight :size="17"/></span><div class="transaction-info"><strong>{{ label(item.kind) }}</strong><time>{{ when(item.createdAt) }}</time></div><span :class="['transaction-amount',{'positive':item.amountMicros>0}]">{{ item.amountMicros>0?'+':'−' }}¥{{ money(Math.abs(item.amountMicros),6) }}</span></div></div>
  </section>
 </div>
</template>
<style scoped>
.account-page{max-width:940px;margin:0 auto;padding:32px 36px 48px;color:#181b2e}.page-heading{display:flex;align-items:center;justify-content:space-between;margin-bottom:28px;gap:18px}h1{font-size:22px;font-weight:650;letter-spacing:-.04em}h2{font-size:15px;font-weight:600}.page-heading p{font-size:13px;color:#72788b;margin-top:7px;max-width:620px}.icon-button{width:34px;height:34px;display:grid;place-items:center;border:1px solid #e2e4ec;border-radius:9px;background:white;color:#73798b;flex-shrink:0}.wallet-panel{background:#fff;border:1px solid #e1e4ee;border-radius:18px;display:grid;grid-template-columns:1.08fr 1fr;overflow:hidden}.wallet-summary{padding:34px;display:flex;flex-direction:column;align-items:flex-start;background:#f1f3fe}.wallet-eyebrow{display:flex;align-items:center;gap:9px;font-size:13px;font-weight:550;color:#6367a0}.balance{font-size:48px;line-height:1.3;font-weight:650;letter-spacing:-.045em;white-space:nowrap;font-variant-numeric:tabular-nums;margin-top:24px}.currency{font-size:25px;font-weight:400;margin-right:6px;color:#6a6f96}.balance-caption{font-size:12px;color:#848aa0;margin-top:3px}.account-identity{display:flex;align-items:center;flex-wrap:wrap;gap:7px;margin-top:auto;padding-top:32px;color:#6f758c;font-size:12px;word-break:break-all}.account-identity button{display:flex;gap:5px;align-items:center;margin-left:6px;color:#7f8599}.signed-out-hint{font-size:12px;color:#7a8095;margin-top:30px}.login-panel,.topup-panel{padding:30px}.login-panel h2{margin-bottom:18px}.login-panel label{display:block;font-size:12px;color:#747a8d;margin:14px 0 7px}input{width:100%;min-width:0;height:40px;border:1px solid #dfe3ed;background:#fbfcff;border-radius:8px;padding:0 11px;font-size:13px;outline:none;color:#242940}input:focus{border-color:#686be3;box-shadow:0 0 0 3px #6366f114}.code-row{display:flex;gap:8px}.code-row input{flex:1;width:0;letter-spacing:.13em}.primary-button,.secondary-button{display:inline-flex;align-items:center;justify-content:center;gap:8px;border-radius:9px;min-height:40px;font-size:13px;font-weight:550;padding:0 15px;white-space:nowrap}.primary-button{background:#5358ce;color:#fff}.primary-button:hover{background:#454bc0}.secondary-button{border:1px solid #e0e3ec;color:#565c74;background:#fff;font-size:12px}.login-button{width:100%;margin-top:18px}.topup-panel{display:flex;flex-direction:column;align-items:flex-start;justify-content:center}.topup-value{font-size:32px;letter-spacing:-.035em;font-weight:600;margin-bottom:20px;font-variant-numeric:tabular-nums}.topup-value span{font-size:18px;color:#8b91a2;margin-right:4px}.topup-panel .primary-button{min-width:180px}.topup-panel p{font-size:12px;color:#8a90a1;line-height:1.8;margin-top:14px;max-width:260px}.wallet-footnote{font-size:12px;color:#969bad;margin:14px 2px 0}.wallet-stats{display:grid;grid-template-columns:repeat(3,1fr);padding:24px 0;border-bottom:1px solid #e5e7ee}.wallet-stats div{padding:0 24px;border-right:1px solid #e5e7ee;display:flex;flex-direction:column;gap:8px}.wallet-stats div:first-child{padding-left:2px}.wallet-stats div:last-child{border:0}.wallet-stats span{font-size:12px;color:#8c92a2}.wallet-stats strong{font-size:18px;font-weight:550;font-variant-numeric:tabular-nums}.history-section{margin-top:30px}.history-heading{display:flex;justify-content:space-between;align-items:center;gap:12px;margin-bottom:16px}.history-filter{display:flex;gap:3px}.history-filter button{font-size:11px;color:#8e94a5;padding:5px 9px;border-radius:6px}.history-filter button.active{background:#e9ebf8;color:#565bbb}.history-empty{display:flex;flex-direction:column;align-items:center;gap:10px;padding:36px 20px;color:#acb1c0;border:1px dashed #dce0eb;border-radius:12px}.history-empty p{font-size:12px}.history-list{background:#fff;border:1px solid #e7e9f0;border-radius:12px;padding:0 18px}.transaction{display:flex;align-items:center;gap:12px;padding:15px 0;border-bottom:1px solid #f0f1f6}.transaction:last-child{border:0}.transaction-icon{width:34px;height:34px;border-radius:10px;display:grid;place-items:center;color:#8b8faa;background:#f0f1f7}.transaction-icon.recharge{color:#4b9072;background:#edf6f0}.transaction-info{display:flex;flex-direction:column;gap:3px}.transaction-info strong{font-size:12px;font-weight:550}.transaction-info time{font-size:11px;color:#a0a5b4}.transaction-amount{margin-left:auto;font-size:13px;font-variant-numeric:tabular-nums;color:#737b90}.transaction-amount.positive{color:#367959}.payment-note{display:flex;align-items:center;gap:8px;padding:12px 15px;margin-top:14px;border-radius:9px;font-size:12px;background:#fff9e9;color:#9b7b37}.payment-note.paid{background:#eaf5ed;color:#407655}.error-message{padding:12px 14px;border-radius:8px;background:#fff1f1;color:#b65c5c;font-size:12px;margin-bottom:16px}.form-error,.form-note{font-size:11px;margin-top:9px;color:#b65c5c}.form-note{color:#6f8c76}button{cursor:pointer;transition:background .15s}button:disabled{opacity:.45;cursor:not-allowed}button:focus-visible{outline:2px solid #818cf8;outline-offset:3px}
:global(.dark .account-page){color:#e4e5ee}:global(.dark .wallet-panel),:global(.dark .history-list),:global(.dark .icon-button){background:#1a1b25;border-color:#30313f}:global(.dark .wallet-summary){background:#202136}:global(.dark input),:global(.dark .secondary-button){background:#21222e;color:#dddfea;border-color:#393b50}:global(.dark .wallet-stats),:global(.dark .wallet-stats div){border-color:#30323f}:global(.dark .transaction){border-color:#2b2c3a}:global(.dark .history-empty){border-color:#37394b}:global(.dark .history-filter button.active){background:#30334c;color:#b7baff}:global(.dark .transaction-icon){background:#2d2f42}:global(.dark .error-message){background:#38272e}:global(.dark .payment-note){background:#352f24}:global(.dark .payment-note.paid){background:#23352c}
@media(max-width:760px){.account-page{padding:24px 20px}.wallet-panel{grid-template-columns:1fr}.wallet-summary{padding:25px}.balance{font-size:42px;margin-top:16px}.history-heading{align-items:flex-start;flex-direction:column}.wallet-stats div{padding:0 12px}.wallet-stats strong{font-size:15px}}@media(prefers-reduced-motion:reduce){button{transition:none}.animate-spin{animation:none}}
</style>
