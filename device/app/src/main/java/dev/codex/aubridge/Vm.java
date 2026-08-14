package dev.codex.aubridge;

import android.accessibilityservice.GestureDescription;
import android.content.Context;
import android.content.Intent;
import android.graphics.Path;
import android.os.Build;
import android.os.Bundle;
import android.os.SystemClock;
import android.view.accessibility.AccessibilityNodeInfo;
import org.json.JSONArray;
import org.json.JSONException;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;

final class Vm {
    private final Context context;private final Capture capture;
    private final Map<String,R> done=new LinkedHashMap<String,R>(128,.75f,true){@Override protected boolean removeEldestEntry(Map.Entry<String,R> e){return size()>128;}};
    private final Set<String> running=new HashSet<>();
    Vm(Context context,Ui ignored,Capture capture){this.context=context;this.capture=capture;}

    JSONArray run(long seq,JSONArray q){String id=q.optString(3,"");synchronized(done){R prior=done.get(id);if(prior!=null)return prior.wire(seq);if(running.contains(id))return new R(7,0,0,0,null).wire(seq);running.add(id);}R result;try{result=execute(q);}catch(Exception e){result=new R(e instanceof Stale?1:e instanceof Timeout?2:e instanceof Bridge.Limit?6:e instanceof Permission?10:5,gen(),0,0,null);}finally{synchronized(done){running.remove(id);}}synchronized(done){done.put(id,result);}return result.wire(seq);}
    private R execute(JSONArray q)throws Exception {
        if(q.length()!=7)throw new Bridge.Limit();long expected=q.getLong(2);String id=q.getString(3);if(!id.matches("[A-Za-z0-9_-]{1,64}"))throw new Bridge.Limit();int deadline=q.getInt(4),budget=q.getInt(5);if(deadline<1||deadline>30_000||budget<0||budget>16)throw new Bridge.Limit();JSONArray rows=q.getJSONArray(6);if(rows.length()<1||rows.length()>32)throw new Bridge.Limit();Ui ui=requireUi();Ui.Scene scene;List<P> plan=new ArrayList<>(rows.length());int mutations=0;synchronized(ui.guard()){scene=ui.snapshot();if(scene.generation!=expected)throw new Stale();for(int i=0;i<rows.length();i++){P p=P.parse(rows.getJSONArray(i),scene,context,capture);plan.add(p);if(p.mutates&&++mutations>budget)throw new Bridge.Limit();}}
        long end=SystemClock.elapsedRealtime()+deadline;int committed=0;boolean guarded=false;String artifact=null;try{for(int i=0;i<plan.size();i++){P p=plan.get(i);if(SystemClock.elapsedRealtime()>end)throw new Fail(i,true);boolean ok;if(p.mutates&&!guarded){synchronized(ui.guard()){if(ui.snapshot().generation!=expected)throw new Stale();ok=p.apply(context,ui,capture,end);guarded=true;}}else ok=p.apply(context,ui,capture,end);if(!ok)throw new Fail(i,false);if(p.mutates)committed++;if(p.artifact!=null)artifact=p.artifact;}}catch(Stale e){if(committed==0)throw e;return new R(3,gen(),committed,0,artifact);}catch(Fail e){return new R(committed>0?3:e.timeout?2:9,gen(),committed,e.at,artifact);}finally{for(P p:plan)p.close();}return new R(0,gen(),committed,-1,artifact);
    }
    private static long gen(){Ui u=Ui.get();return u==null?0:u.generation();}
    private static Ui requireUi()throws Unsupported{Ui u=Ui.get();if(u==null)throw new Unsupported();return u;}

    static final class R {final int code;final long g;final int mutations,at;final String artifact;R(int code,long g,int mutations,int at,String artifact){this.code=code;this.g=g;this.mutations=mutations;this.at=at;this.artifact=artifact;}JSONArray wire(long seq){JSONArray a=new JSONArray().put(seq).put(code).put(g).put(mutations);if(at>=0)a.put(at);else if(artifact!=null)a.put(org.json.JSONObject.NULL);if(artifact!=null)a.put(artifact);return a;}}
    static final class P implements AutoCloseable {
        final String name;final AccessibilityNodeInfo node;final String text;final int arg;final Pred pred;final JSONArray points;final boolean mutates;String artifact;
        P(String name,AccessibilityNodeInfo node,String text,int arg,Pred pred,JSONArray points,boolean mutates){this.name=name;this.node=node;this.text=text;this.arg=arg;this.pred=pred;this.points=points;this.mutates=mutates;}
        static P parse(JSONArray a,Ui.Scene s,Context context,Capture capture)throws Exception{if(a.length()<1)throw new Bridge.Limit();String n=a.getString(0);switch(n){
            case"tap":exact(a,2);return ref(n,a,s,true);case"long":exact(a,2);return ref(n,a,s,true);case"text":exact(a,3);String t=bounded(a.getString(2),8192);return new P(n,copy(s,a.getInt(1)),t,0,null,null,true);
            case"scroll":exact(a,3);String d=a.getString(2);int dir="up".equals(d)||"left".equals(d)?-1:"down".equals(d)||"right".equals(d)?1:0;if(dir==0)throw new Bridge.Limit();return new P(n,copy(s,a.getInt(1)),null,dir,null,null,true);
            case"key":exact(a,2);String k=a.getString(1);if(!k.matches("back|home|recents|notifications|enter")||(k.equals("enter")&&Build.VERSION.SDK_INT<30))throw new Unsupported();return new P(n,null,k,0,null,null,true);
            case"gesture":exact(a,2);JSONArray pts=a.getJSONArray(1);if(pts.length()<2||pts.length()>16)throw new Bridge.Limit();for(int i=0;i<pts.length();i++){JSONArray p=pts.getJSONArray(i);if(p.length()!=3||p.getInt(0)<0||p.getInt(0)>65_535||p.getInt(1)<0||p.getInt(1)>65_535||p.getInt(2)<0||p.getInt(2)>30_000)throw new Bridge.Limit();}return new P(n,null,null,0,null,pts,true);
            case"wait":exact(a,3);int ms=a.getInt(2);if(ms<0||ms>30_000)throw new Bridge.Limit();return new P(n,null,null,ms,Pred.parse(a.getJSONArray(1),s),null,false);
            case"assert":exact(a,2);return new P(n,null,null,0,Pred.parse(a.getJSONArray(1),s),null,false);
            case"launch":exact(a,2);String pkg=bounded(a.getString(1),255);Intent launch=context.getPackageManager().getLaunchIntentForPackage(pkg);if(launch==null)throw new Unsupported();return new P(n,null,pkg,0,null,null,true);
            case"capture":exact(a,2);if(!"screen".equals(a.getString(1))||Build.VERSION.SDK_INT<30)throw new Unsupported();return new P(n,null,"screen",0,null,null,false);
            case"camera":if(a.length()!=2&&a.length()!=4)throw new Bridge.Limit();String facing=a.getString(1);if(!"rear".equals(facing)&&!"front".equals(facing)&&!facing.isEmpty())throw new Bridge.Limit();if(a.length()==4){int width=a.getInt(2),height=a.getInt(3);if(width<160||width>4096||height<160||height>4096)throw new Bridge.Limit();facing=facing+"|"+width+"x"+height;}return new P(n,null,facing,0,null,null,true);
            case"microphone":exact(a,2);int seconds=a.getInt(1);if(seconds<1||seconds>30)throw new Bridge.Limit();return new P(n,null,"microphone",seconds,null,null,true);
            case"screen_record":exact(a,2);int duration=a.getInt(1);if(duration<1||duration>30)throw new Bridge.Limit();return new P(n,null,"screen_record",duration,null,null,true);
            case"notification_open":case"notification_dismiss":case"notification_action":exact(a,2);return new P(n,null,bounded(a.getString(1),256),0,null,null,true);
            default:throw new Unsupported();}}
        private static P ref(String n,JSONArray a,Ui.Scene s,boolean mut)throws Bridge.Limit{ return new P(n,copy(s,a.optInt(1,-1)),null,0,null,null,mut); }
        boolean apply(Context context,Ui ui,Capture capture,long end)throws Exception{switch(name){
            case"tap":return node.performAction(AccessibilityNodeInfo.ACTION_CLICK);case"long":return node.performAction(AccessibilityNodeInfo.ACTION_LONG_CLICK);case"text":Bundle b=new Bundle();b.putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE,text);return node.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT,b);case"scroll":return node.performAction(arg<0?AccessibilityNodeInfo.ACTION_SCROLL_BACKWARD:AccessibilityNodeInfo.ACTION_SCROLL_FORWARD);
            case"key":return key(ui,text);case"gesture":return gesture(ui,points);case"wait":long until=Math.min(end,SystemClock.elapsedRealtime()+arg);do{if(pred.test(ui))return true;Thread.sleep(50);}while(SystemClock.elapsedRealtime()<until);return false;case"assert":return pred.test(ui);
            case"launch":Intent i=context.getPackageManager().getLaunchIntentForPackage(text);if(i==null)return false;i.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);context.startActivity(i);return true;case"capture":artifact=capture.screen();return artifact!=null;case"camera":artifact=capture.camera(text);return artifact!=null;case"microphone":artifact=capture.microphone(arg);return artifact!=null;case"screen_record":artifact=capture.screenRecord(arg);return artifact!=null;case"notification_open":return Notify.open(text);case"notification_dismiss":return Notify.dismiss(text);case"notification_action":return Notify.action(text);default:return false;}}
        @Override public void close(){if(node!=null)node.recycle();}
        private static boolean key(Ui ui,String key){switch(key){case"back":return ui.performGlobalAction(Ui.GLOBAL_ACTION_BACK);case"home":return ui.performGlobalAction(Ui.GLOBAL_ACTION_HOME);case"recents":return ui.performGlobalAction(Ui.GLOBAL_ACTION_RECENTS);case"notifications":return ui.performGlobalAction(Ui.GLOBAL_ACTION_NOTIFICATIONS);case"enter":AccessibilityNodeInfo root=ui.getRootInActiveWindow();if(root==null)return false;AccessibilityNodeInfo focus=root.findFocus(AccessibilityNodeInfo.FOCUS_INPUT);boolean ok=focus!=null&&focus.performAction(AccessibilityNodeInfo.AccessibilityAction.ACTION_IME_ENTER.getId());if(focus!=null)focus.recycle();root.recycle();return ok;default:return false;}}
        private static boolean gesture(Ui ui,JSONArray pts)throws JSONException{Path path=new Path();JSONArray p=pts.getJSONArray(0);path.moveTo(p.getInt(0),p.getInt(1));long duration=1;for(int i=1;i<pts.length();i++){p=pts.getJSONArray(i);path.lineTo(p.getInt(0),p.getInt(1));duration=Math.max(duration,p.getLong(2));}GestureDescription g=new GestureDescription.Builder().addStroke(new GestureDescription.StrokeDescription(path,0,Math.min(duration,30_000))).build();return ui.dispatchGesture(g,null,null);}
        private static AccessibilityNodeInfo copy(Ui.Scene s,int id)throws Bridge.Limit{Ui.N n=s.ref(id);if(n==null)throw new Bridge.Limit();return AccessibilityNodeInfo.obtain(n.node);}
        private static void exact(JSONArray a,int n)throws Bridge.Limit{if(a.length()!=n)throw new Bridge.Limit();}
    }
    static final class Pred {final String kind,label;final Integer ref;final long generation;Pred(String kind,String label,Integer ref,long generation){this.kind=kind;this.label=label;this.ref=ref;this.generation=generation;}
        static Pred parse(JSONArray a,Ui.Scene s)throws Exception{if(a.length()!=2)throw new Bridge.Limit();String k=a.getString(0);switch(k){case"text":return new Pred(k,bounded(a.getString(1),1024),null,0);case"generation_after":return new Pred(k,null,null,a.getLong(1));case"exists":case"missing":Object v=a.get(1);if(v instanceof Number){int r=((Number)v).intValue();if(s.ref(r)==null)throw new Bridge.Limit();return new Pred(k,null,r,0);}JSONArray m=(JSONArray)v;if(m.length()!=2||!"label".equals(m.getString(0)))throw new Bridge.Limit();return new Pred(k,bounded(m.getString(1),1024),null,0);default:throw new Unsupported();}}
        boolean test(Ui ui){Ui.Scene s=ui.snapshot();boolean v;if("text".equals(kind))return s.label(label);if("generation_after".equals(kind))return s.generation>generation;if(ref!=null)v=s.ref(ref)!=null;else v=s.label(label);return"exists".equals(kind)?v:!v;}}
    private static String bounded(String s,int max)throws Bridge.Limit{if(s.getBytes(StandardCharsets.UTF_8).length>max)throw new Bridge.Limit();return s;}
    static final class Stale extends Exception{}static final class Timeout extends Exception{}static final class Unsupported extends Exception{}static final class Permission extends Exception{}static final class Fail extends Exception{final int at;final boolean timeout;Fail(int at,boolean timeout){this.at=at;this.timeout=timeout;}}
}
