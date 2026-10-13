"use client";
import { useState } from "react";
export function ConsentForm({authorizationId,clientName,redirectUri,email,scope,userId,workspaceId,workspaceName,membershipRole}:{authorizationId:string;clientName:string;redirectUri:string;email:string;scope:string;userId:string;workspaceId:string;workspaceName:string;membershipRole:"owner"|"reviewer"}) {
  const [allowWrites,setAllowWrites]=useState(true); const [busy,setBusy]=useState(false); const [message,setMessage]=useState("");
  async function decide(approve:boolean) {
    setBusy(true);setMessage("");
    try {
      const response=await fetch("/api/oauth/consent",{method:"POST",headers:{"Content-Type":"application/json"},body:JSON.stringify({authorizationId,approve,allowWrites,expectedWorkspaceId:workspaceId,expectedUserId:userId})});
      const data=await response.json();if(!response.ok||!data.redirect_url) throw new Error();
      window.location.assign(data.redirect_url);
    } catch {setMessage("Authorization could not be completed. Restart the connection from your agent.");setBusy(false);}
  }
  return <div className="login-card"><span className="eyebrow">Agent authorization</span><h1>Connect {clientName}?</h1>
    <p>Signed in as <strong>{email}</strong>, {membershipRole === "owner" ? "an owner" : "a member"} in <strong>{workspaceName}</strong>.</p><p>This client will be able to read every project in this shared workspace. Your teammates can see the reasoning it captures.</p>
    <label className="permission-choice"><input type="checkbox" checked={allowWrites} onChange={e=>setAllowWrites(e.target.checked)}/> Allow reasoning capture</label>
    <p>This connection alone does not authorize commitments. A workspace owner can separately grant this named agent authority to adopt, revise, or withdraw commitments in a specific project. It cannot invite people or manage projects. You can disconnect it in Agent connections; removing your membership also removes its access.</p>
    <small>Client names are supplied by the requesting application. Return address: {redirectUri}</small><small>Identity scopes: {scope||"None"}</small>
    <div className="detail-actions"><button className="button primary" onClick={()=>decide(true)} disabled={busy}>Authorize agent</button>
      <button className="button secondary" onClick={()=>decide(false)} disabled={busy}>Deny</button></div>
    {message&&<p role="alert">{message}</p>}</div>;
}
