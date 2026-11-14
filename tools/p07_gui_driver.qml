import QtQuick
Item {
 id: driver
 property var window: productionWindow
 parent: window ? window.contentItem : null
 property var config: @CONFIG@
 property int step: 0
 property int axis: 0
 property double since: Date.now()
 property double lastTick: Date.now()
 property double maxTickGap: 0
 property int stateChanges: 0
 property int hiddenChanges: 0
 property int logChanges: 0
 property int hiddenLogChanges: 0
 property string hiddenCursor: ""
 property double cancelStarted: 0
 property double stopStarted: 0
 property double cancelMs: 0
 property double stopMs: 0
 function require(ok, why) { if (!ok) { console.error("P07 FAIL " + why); Qt.exit(3); throw Error(why) } }
 function find(item,name) {
  if(item.objectName===name) return item
  if(item.children) for(let child of item.children) { const found=find(child,name); if(found) return found }
  return null
 }
 function click(name) {
  const button=find(window.contentItem,name)
  require(button && button.visible && button.enabled,"button "+name)
  button.clicked()
 }
 function send(value) { window.send(value) }
 function advance(next) { console.log("P07 STEP " + next); step=next; since=Date.now() }
 Connections { target: window; function onStateChanged() { driver.stateChanges++ } }
 Connections { target: window ? find(window.contentItem,"logsView") : null; function onPageChanged() { driver.logChanges++ } }
 Timer {
  interval: 25; repeat: true; running: true
  onTriggered: {
   const now=Date.now(); maxTickGap=Math.max(maxTickGap,now-lastTick); lastTick=now
   require(now-driver.since < 15000,"step deadline "+step+" state "+JSON.stringify(window.state))
   const s=window.state
   if(step===0 && s.ready) { send({command:"set_engine",engine:config.engine}); advance(1) }
   else if(step===1 && s.engine===config.engine) { send({command:"import_source",name:"P07",url:config.source,local_exception:true,route:"direct"}); advance(2) }
   else if(step===2 && s.total===65 && !s.busy) {
    const profile=s.profiles.filter(p=>p.protocol==="socks5")[0]; require(!!profile,"LocalSocks profile")
    send({command:"select_profile",id:profile.id,revision:s.revision}); advance(3)
   }
   else if(step===3 && s.selected_profile!==null) { send({command:"set_probe_target",url:"https://198.18.0.1:"+(18080+axis)+"/",expected_status:204}); advance(4) }
   else if(step===4 && s.probe_custom) { click("mainConnectButton"); advance(5) }
   else if(step===5 && (s.status==="Connected" || s.status==="Limited")) {
    const expected=["http","tls","transport"][axis]
    require(s.probe_result && s.probe_result.kind===expected,"strict result "+axis+JSON.stringify(s.probe_result))
    require(axis!==0 || (s.status==="Connected" && s.probe_result.http_status===204),"HTTPS 204")
    require(axis===0 || s.status==="Limited","negative must not connect")
    console.log("P07 STRICT "+JSON.stringify({axis:axis,result:s.probe_result,status:s.status}))
    click("mainConnectButton"); advance(6)
   }
   else if(step===6 && s.status==="Disconnected" && !s.can_stop) {
    axis++; if(axis<3) advance(3)
    else { send({command:"set_probe_target",url:"https://198.18.0.1:18083/",expected_status:204}); advance(7) }
   }
   else if(step===7) { click("mainConnectButton"); advance(8) }
   else if(step===8 && s.status==="Checking" && now-since>200) {
    send({command:"p07_logs"}); click("settingsButton"); click("settingsDiagnosticsButton"); advance(9)
   }
   else if(step===9) { click("batchCheckButton"); advance(10) }
   else if(step===10 && s.batch.busy && now-since>200) {
    send({command:"import_text",name:"P07-busy",text:"socks5://127.0.0.1:1080"}); advance(11)
   }
   else if(step===11 && s.source_error==="busy") {
    require(s.status==="Checking" && s.batch.busy,"actual concurrency")
    click("logsRefresh"); advance(12)
   }
   else if(step===12 && now-since>150) {
    const view=find(window.contentItem,"logsView"); require(view.page && view.page.events.length<=256,"bounded log page")
    hiddenCursor=view.page.cursor; hiddenChanges=stateChanges; hiddenLogChanges=logChanges
    window.hide(); advance(13)
   }
   else if(step===13 && now-since>600) {
    const view=find(window.contentItem,"logsView")
    require(!window.visible,"window hidden")
    require(logChanges===hiddenLogChanges && view.page.cursor===hiddenCursor,"hidden UI processed log page")
    require(stateChanges-hiddenChanges<100,"hidden snapshot-per-line")
    console.log("P07 HIDDEN "+JSON.stringify({duration_ms:now-since,state_changes:stateChanges-hiddenChanges,log_page_changes:logChanges-hiddenLogChanges,cursor:hiddenCursor}))
    window.show(); advance(14)
   }
   else if(step===14) { cancelStarted=now; click("batchCancelButton"); advance(15) }
   else if(step===15 && !s.batch.busy && s.batch.cancelled) {
    cancelMs=now-cancelStarted; console.log("P07 CANCEL_ACK "+JSON.stringify(s.batch)); advance(16)
   }
   else if(step===16 && now-since>200) {
    console.log("P07 BATCH_DRAINED")
    advance(161)
   }
   else if(step===161 && now-since>300) {
    click("homeButton"); click("homeButton"); stopStarted=now; click("mainConnectButton"); advance(17)
   }
   else if(step===17 && s.status==="Disconnected" && !s.can_stop) {
    stopMs=now-stopStarted; console.log("P07 STOP_ACK"); advance(18)
   }
   else if(step===18 && now-since>4500) {
    console.log("P07 GUI_PASS "+JSON.stringify({cancel_snapshot_ms:cancelMs,stop_snapshot_ms:stopMs,max_driver_tick_gap_ms:maxTickGap,state_changes:stateChanges}))
    send({command:"shutdown"}); advance(19)
   }
  }
 }
}
