// First-party test specimen; this module is parsed, never executed.
var identity, readFlag;
function metadata(t) {
  let e = t.requestMeta?.["x-codex-turn-metadata"];
  if (typeof e == "string") try { e = JSON.parse(e); } catch { return; }
  if (!(e == null || typeof e != "object" || Array.isArray(e))) return e;
}
async function policy() {
  if (identity == null) throw new Error("Browser request-header policy requires caller identity.");
  return await identity, readFlag("codex_browser_use_agent_request_header");
}
var Browser = class extends Base {
  constructor(r,n,o,i,s) {
    super(r);
    this.apiTransport=r;this.getTurnMetadata=o;this.turnEndedTracker=i;
    this.readRequestHeaderEnabled=s;
    this.registerRequestHandlerObject(n),this.addEventListener("onPageEvent",a=>{
      this.pendingPageEvents.push(a);
    });
  }
  async getInfo() {
    let r=await this.sendSessionRequest("getInfo",{});
    return this.clientInfo=r,r;
  }
  async sendSessionRequest(r,n) {
    let o=this.getSessionParams();
    if(r!=="getInfo"&&r!=="getUserTabs"&&(this.lastSessionParams=o),
      this.clientInfo?.type==="extension"&&
      this.clientInfo.agentRequestHeaderEnabled!==void 0&&
      r!=="getInfo"&&this.endingTurnId!==o.turn_id&&this.readRequestHeaderEnabled!=null) {
      let i=this.requestHeaderEnabled||this.clientInfo.agentRequestHeaderEnabled===!0||
        await this.readRequestHeaderEnabled();
      if(i&&typeof this.clientInfo.agentRequestHeaderEnabled!="boolean")
        throw new Error("This browser requires agent request headers. Update the Chrome extension before continuing.");
      this.requestHeaderEnabled||=i,o.agent_request_header_enabled=i,
      this.lastSessionParams?.session_id===o.session_id&&
      this.lastSessionParams.turn_id===o.turn_id&&
      (this.lastSessionParams.agent_request_header_enabled=i);
    }
    return this.clientInfo!=null&&this.turnEndedTracker!=null&&
      this.endingTurnId!==o.turn_id&&
      this.turnEndedTracker.track({session_id:o.session_id,turn_id:o.turn_id},this.turnEnded),
      this.sendRequest(r,{...n,...o});
  }
};
function refresh() {
  return new Browser(r,this.clientApi,()=>metadata(this.runtime),this.turnEndedTracker,policy);
}
