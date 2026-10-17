<script setup lang="ts">
import {computed,onMounted,ref} from 'vue';
import {isTauri} from '@tauri-apps/api/core';
import {useRouter} from 'vue-router';
import {Sparkles,ArrowUpRight,Check,Plus,LoaderCircle} from 'lucide-vue-next';
import {usePlatformAccountStore} from '../stores/usePlatformAccountStore';
import {getAgentStatuses,type AgentInfo} from '../api/agents';
import {getAgentModelMap,assignModelToAgent,removeModelFromAgent} from '../api/models';
const MODEL='runjam-builtin',router=useRouter(),account=usePlatformAccountStore();
const agents=ref<AgentInfo[]>([]),assigned=ref<string[]>([]),assigning=ref(''),error=ref('');
const installed=computed(()=>agents.value.filter(a=>a.installed));
const balance=computed(()=>new Intl.NumberFormat(undefined,{minimumFractionDigits:2,maximumFractionDigits:4}).format((account.account?.wallet.availableMicros||0)/1000000));
async function load(){
 if(!isTauri())return;
 try{const [list,map]=await Promise.all([getAgentStatuses(),getAgentModelMap()]);agents.value=list;assigned.value=map.filter(m=>m.model_id===MODEL).map(m=>m.agent_id);}catch(e){error.value=String(e);}
}
async function assign(id:string){
 if(assigning.value)return;
 if(!account.loggedIn){void router.push('/settings/account');return;}
 assigning.value=id;error.value='';
 try{if(assigned.value.includes(id))await removeModelFromAgent(id,MODEL);else await assignModelToAgent(id,MODEL,true);await load();}
 catch(e){error.value=String(e);}finally{assigning.value='';}
}
onMounted(()=>{void account.refresh();void load();});
</script>
<template>
 <section class="builtin-card">
  <div class="builtin-top"><div class="builtin-symbol"><Sparkles :size="22"/></div><div class="builtin-title"><h3>{{ $t('account.builtinTitle') }}</h3><p>{{ $t('account.builtinDesc') }}</p></div><button class="account-link" @click="router.push('/settings/account')">{{ account.loggedIn?$t('account.manage'):$t('account.login') }}<ArrowUpRight :size="14"/></button></div>
  <div class="builtin-bottom"><div class="balance-label"><template v-if="account.loggedIn"><span>{{ $t('account.available') }}</span><strong>¥{{ balance }}</strong></template><span v-else>{{ $t('account.installHint') }}</span></div><div class="agent-actions"><button v-for="agent in installed" :key="agent.id" :class="['agent-chip',{'enabled':assigned.includes(agent.id)}]" :aria-pressed="assigned.includes(agent.id)" :disabled="!!assigning||account.catalog?.hosted.available===false" :title="$t('account.assign')" @click="assign(agent.id)"><LoaderCircle v-if="assigning===agent.id" :size="12" class="animate-spin"/><Check v-else-if="assigned.includes(agent.id)" :size="12"/><Plus v-else :size="12"/>{{ agent.display_name }}</button><span v-if="installed.length===0" class="no-agents">{{ $t('account.noAgents') }}</span></div></div>
  <p v-if="error" role="alert" class="card-error">{{ error }}</p>
  <p v-else-if="account.catalog?.hosted.available===false" class="card-error">{{ $t('account.notAvailable') }}</p>
 </section>
</template>
<style scoped>
.builtin-card{border:1px solid #dce0f2;border-radius:14px;overflow:hidden;background:#fff;margin-bottom:26px}.builtin-top{display:flex;align-items:center;gap:13px;padding:22px 22px 20px}.builtin-symbol{width:43px;height:43px;border-radius:12px;background:#eeeffb;color:#656bcd;display:grid;place-items:center;flex-shrink:0}.builtin-title{flex:1;min-width:0}.builtin-title h3{font-size:14px;font-weight:650;color:#2f3354}.builtin-title p{font-size:12px;color:#8a90a6;line-height:1.6;margin-top:5px}.account-link{display:flex;align-items:center;gap:6px;font-size:12px;font-weight:550;color:#676ac5;flex-shrink:0}.builtin-bottom{display:flex;align-items:center;justify-content:space-between;gap:14px;border-top:1px solid #edf0f8;background:#f8f9ff;padding:13px 22px}.balance-label{display:flex;gap:10px;align-items:center;white-space:nowrap}.balance-label span{font-size:11px;color:#9499b0}.balance-label strong{font-size:14px;font-weight:600;font-variant-numeric:tabular-nums;color:#555b99}.agent-actions{display:flex;flex-wrap:wrap;gap:7px;justify-content:flex-end}.agent-chip{display:flex;align-items:center;gap:4px;font-size:10px;font-weight:500;padding:5px 8px;border-radius:6px;border:1px solid #e2e6f0;color:#9297ac;background:white}.agent-chip.enabled{background:#e9ecff;border-color:#ced3f5;color:#6066c1}.no-agents{font-size:11px;color:#a1a6b9}.card-error{font-size:11px;color:#b86f6f;padding:10px 22px}button{cursor:pointer}button:disabled{opacity:.45;cursor:not-allowed}button:focus-visible{outline:2px solid #818cf8;outline-offset:3px}:global(.dark .builtin-card){background:#1b1c27;border-color:#383b57}:global(.dark .builtin-bottom){background:#222435;border-color:#30334a}:global(.dark .builtin-title h3){color:#dadcf1}:global(.dark .builtin-symbol){background:#30324b;color:#a5aafa}:global(.dark .agent-chip){background:#282a3a;border-color:#3b3f58}:global(.dark .agent-chip.enabled){background:#34395b;color:#b4baff}:global(.dark .balance-label strong){color:#b4b9f3}@media(max-width:700px){.builtin-top{align-items:flex-start;flex-wrap:wrap}.account-link{margin-left:56px}.builtin-bottom{align-items:flex-start;flex-direction:column}.agent-actions{justify-content:flex-start}}
</style>
