export default {
  async fetch(request, env, ctx) {
    const url = new URL(request.url);
    const targetUrl = url.searchParams.get("url");
    if (!targetUrl) return new Response("?url=github release url", {status:400});
    if(!targetUrl.startsWith("https://github.com/peintune/runjam/releases/download/")){
      return new Response("illegal url",{status:403})
    }
    const res = await fetch(targetUrl, {
      headers:{"User-Agent":"Mozilla/5.0"}
    });
    return new Response(res.body, {
      headers:{
        "Content‑Disposition":res.headers.get("Content‑Disposition"),
        "Content‑Type":res.headers.get("Content‑Type")
      }
    })
  }
}