import {invoke,isTauri} from '@tauri-apps/api/core';
import {t} from '../i18n';
function call<T>(command:string,args?:Record<string,unknown>):Promise<T>{
 if(!isTauri())return Promise.reject(new Error(t('account.desktopRequired')));
 return invoke<T>(command,args);
}
export interface PlatformWallet {balanceMicros:number;availableMicros:number;reservedMicros:number;rechargedMicros:number;spentMicros:number;refundedMicros?:number}
export interface PlatformAccount {user:{id:string;email:string};wallet:PlatformWallet}
export interface PlatformCatalog {paymentAvailable:boolean;hosted:{available:boolean};products:Record<string,{name?:string;amountMicros:number}>}
export interface PlatformSnapshot {account:PlatformAccount|null;catalog:PlatformCatalog}
export interface LedgerItem {kind:'recharge'|'usage'|'refund';amountMicros:number;createdAt:number;detail?:{label?:string;inputTokens?:number;outputTokens?:number}}
// Sessions live in native OS credentials. None of these IPC responses contains a token.
export const platformAccount={
 status:()=>call<PlatformSnapshot>('platform_status'),
 sendCode:(email:string)=>call<{sent:boolean}>('platform_send_code',{email}),
 login:(email:string,code:string)=>call<PlatformSnapshot>('platform_login',{email,code}),
 logout:()=>call<{ok:boolean}>('platform_logout'),
 ledger:()=>call<{items:LedgerItem[]}>('platform_ledger'),
 checkout:()=>call<{orderId:string}>('platform_checkout'),
 order:(orderId:string)=>call<{orderId:string;status:string}>('platform_order',{orderId}),
};
