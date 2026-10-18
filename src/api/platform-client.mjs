// Generated from runjam_web/clients/platform-client.mjs. Edit the shared source.
// Shared transport contract. No merchant, database or model credential belongs
// here. Sessions stay in memory; hosts may supply their own secure token store.
const products=new Set(['musicplus','pasteguana','runjam']);
function diagnosticReport(body) {
  if(!Array.isArray(body?.errors))return body;
  const allowed=new Set(['code','status','http_status','duration_ms','retry_count','agent_type','cli','model','stop_reason','stopReason','update_type','platform','arch']);
  return {...body,errors:body.errors.map(item=>{
    const category=/^[a-z0-9_:-]{1,32}$/.test(item?.category||'')?item.category:'unknown';
    const context=Object.fromEntries(Object.entries(item?.context||{}).filter(([k,v])=>allowed.has(k)&&
      (typeof v==='boolean'||(typeof v==='number'&&Number.isFinite(v))||(typeof v==='string'&&/^[a-zA-Z0-9_.:/-]{1,100}$/.test(v)))));
    return {level:item?.level,category,message:'客户端诊断：'+category,context,created_at:item?.created_at};
  })};
}
export class PlatformClient {
  constructor(product,options={}) {
    if(!products.has(product))throw new Error('产品不存在');
    this.product=product;this.fetch=options.fetch||globalThis.fetch.bind(globalThis);
    this.base='https://supabase-proxy.runjam.app/functions/v1/platform/'+product;
    this.reports=this.base;
    this.token='';this.telemetryEnabled=false;
  }
  setToken(token){this.token=typeof token==='string'?token:'';}
  setTelemetryEnabled(value){this.telemetryEnabled=value===true;}
  async request(path,{body,auth=false,report=false}={}) {
    if(auth&&!this.token)throw new Error('请先登录账号');
    const response=await this.fetch((report?this.reports:this.base)+path,{method:body?'POST':'GET',
      headers:{...(body?{'Content-Type':'application/json'}:{}),...(auth?{Authorization:'Bearer '+this.token}:{})},
      ...(body?{body:JSON.stringify(body)}:{}),signal:AbortSignal.timeout(15000),redirect:'error',credentials:'omit'});
    let result;try{result=await response.json();}catch{throw new Error('服务响应无效，请稍后重试');}
    if(!response.ok){if(auth&&response.status===401)this.token='';throw new Error(typeof result.error==='string'?result.error:'服务暂时不可用');}
    return result;
  }
  sendCode(email){return this.request('/auth/code',{body:{email}});}
  async login(email,code){const result=await this.request('/auth/verify',{body:{email,code}});this.setToken(result.token);return result;}
  async logout(){try{return await this.request('/auth/logout',{body:{},auth:true});}finally{this.token='';}}
  catalog(){return this.request('/catalog');}
  account(){return this.request('/account',{auth:true});}
  ledger(){return this.request('/ledger',{auth:true});}
  checkout(sku,machineId){
    if(machineId&&sku!=='pro')return Promise.reject(new Error('本机购买仅支持 Pro，充值请使用账号'));
    return machineId?this.request('/pro/checkout',{body:{machineId}}):this.request('/checkout',{body:{sku},auth:true});
  }
  restore(machineId){return this.request('/pro/restore',{body:{machineId}});}
  redeem(code,machineId){return this.request('/redeem',{body:{code,machineId}});}
  registerInstallation(body){return this.telemetryEnabled?this.request('/telemetry/register',{body,report:true}):Promise.resolve({disabled:true});}
  events(body){return this.telemetryEnabled?this.request('/telemetry/events',{body,report:true}):Promise.resolve({disabled:true});}
  errors(body){return this.telemetryEnabled?this.request('/telemetry/errors',{body:diagnosticReport(body),report:true}):Promise.resolve({disabled:true});}
  updates(current=''){return this.request('/updates/latest?current='+encodeURIComponent(current));}
  announcements(current=''){return this.request('/announcements?current='+encodeURIComponent(current));}
  releases(){return this.request('/releases');}
  feedback(body){return this.request('/feedback',{body,report:true});}
}
