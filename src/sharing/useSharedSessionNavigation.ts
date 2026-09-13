import {useEffect} from "react";
import {isShareSurface} from "../ipc/shareBase";
import {useTermStore} from "../store/termStore";
import {selectedSharedSession,sharedSessionUrl} from "./sessionNavigation";

export function useSharedSessionNavigation() {
  useEffect(()=>{
    if(!isShareSurface)return;
    let applying=false;
    const restore=()=>{
      const state=useTermStore.getState();if(!state.treeLoaded)return;
      const id=selectedSharedSession();
      applying=true;
      try {
        if(id && state.sessions.some(session=>session.id===id))state.openSession(id);
        else for(const tab of state.openTabs)useTermStore.getState().moveTabToBackground(tab);
      }finally{applying=false}
    };
    const stop=useTermStore.subscribe((state,previous)=>{
      if(applying)return;
      if(state.treeLoaded && (!previous.treeLoaded || state.sessions!==previous.sessions)){restore();return;}
      if(state.treeLoaded && state.activeSessionId!==previous.activeSessionId && state.activeSessionId!==selectedSharedSession()) {
        history.pushState(null,"",sharedSessionUrl(state.activeSessionId));
      }
    });
    window.addEventListener("popstate",restore);restore();
    return ()=>{stop();window.removeEventListener("popstate",restore)};
  },[]);
}
