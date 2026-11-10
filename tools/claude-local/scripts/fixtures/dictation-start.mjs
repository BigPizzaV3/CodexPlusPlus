// 保留实际远程版本的 await、状态清理与 generation 分支；其余依赖使用无设备 stub。
export const recorderFixture = `
class AudioRecorder{
  constructor(context){this.context=context;this.state="inactive";}
  async startWithStream(t){
    try{await this.buildCaptureGraph(t);this.state="recording";}
    catch(error){this.state="error";throw error;}
  }
  setupPcmCapture(){this.scriptProcessorNode.onaudioprocess=e=>{};}
  async buildCaptureGraph(){
    let r=this.context;
    if(this.audioContext=r,r.state==="suspended"&&await r.resume(),this.audioContext!==r)
      throw Error("AudioContext was replaced during setup.");
    if(r.state!=="running")throw Error("AudioContext could not be started");
  }
}
`;

export const facadeFixture = `
let eu=false,sk=0;
function Ar(){let e=fD.getState();e.setLastStopReason(null),e.setError("Dictation couldn't load. Check your connection and try again.");}
var Vf={async start(...e){let t=e[2],n=fD.getState(),r=n.isMouseHolding&&n.ownerId!==t;
if(n.status==="recording"||n.status==="connecting")return;
eu=true;n.setOwnerId(t);XS(t);let a=++sk;try{WM();
let n=fD.getState().startIsWaiting,c;
try{c=await Tl()}catch(e){report({context:"voice machinery chunk load failed on dictation start()"});eu&&Ar();return}
if(!eu)return;await c.dictationSession.start(...e);let u=fD.getState();
if(u.ownerId===t&&!r&&u.isMouseHolding&&u.status!=="idle"&&!u.isPushToTalk)u.setIsPushToTalk(true);
}finally{a===sk&&(XS(null),fD.getState().startIsWaiting&&Vf.stop())}},
stop(...e){eu=false;XS(null);session.stop(...e);}};
`;
