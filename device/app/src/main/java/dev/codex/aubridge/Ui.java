package dev.codex.aubridge;

import android.accessibilityservice.AccessibilityService;
import android.graphics.Rect;
import android.view.accessibility.AccessibilityEvent;
import android.view.accessibility.AccessibilityNodeInfo;
import org.json.JSONArray;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.List;
import java.util.Locale;

public final class Ui extends AccessibilityService {
    private static volatile Ui instance;
    private final Object guard=new Object();
    private long generation=1;
    private Scene scene;

    static Ui get(){return instance;}
    Object guard(){return guard;}
    @Override protected void onServiceConnected(){instance=this;invalidate();}
    @Override public void onAccessibilityEvent(AccessibilityEvent event){if(shouldInvalidate(event.getEventType(),event.getContentChangeTypes()))invalidate();}
    @Override public void onInterrupt(){}
    @Override public void onDestroy(){if(instance==this)instance=null;clear();super.onDestroy();}
    long generation(){synchronized(guard){return generation;}}
    private void invalidate(){synchronized(guard){generation++;clearLocked();}}
    private void clear(){synchronized(guard){clearLocked();}}
    private void clearLocked(){if(scene!=null){scene.recycle();scene=null;}}

    static boolean shouldInvalidate(int type,int changes){
        if(type!=AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED)return true;
        int labels=AccessibilityEvent.CONTENT_CHANGE_TYPE_TEXT|AccessibilityEvent.CONTENT_CHANGE_TYPE_CONTENT_DESCRIPTION|AccessibilityEvent.CONTENT_CHANGE_TYPE_STATE_DESCRIPTION;
        return changes==AccessibilityEvent.CONTENT_CHANGE_TYPE_UNDEFINED||(changes&~labels)!=0;
    }

    Scene snapshot(){synchronized(guard){if(scene==null)scene=build();return scene;}}
    JSONArray observe(long seq,String base,int detail)throws Bridge.Limit {Scene s=snapshot();if(base!=null&&base.equals(Long.toString(s.generation)))return new JSONArray().put(seq).put(0).put(s.generation);int limit=detail==0?Math.min(s.frontier,64):Math.min(s.nodes.size(),256),bytes=0;JSONArray rows=new JSONArray();for(int i=0;i<limit;i++){N n=s.nodes.get(i);int cost=escaped(n.label)+24;if(detail==0&&rows.length()>0&&bytes+cost>3500)break;rows.put(n.wire());bytes+=cost;}s.exposed=Math.min(s.frontier,rows.length());return new JSONArray().put(seq).put(0).put(s.generation).put(s.pkg).put(rows);}

    private Scene build(){AccessibilityNodeInfo root=getRootInActiveWindow();if(root==null)return new Scene(generation,"",new ArrayList<>(),0);String pkg=root.getPackageName()==null?"":root.getPackageName().toString();ArrayDeque<AccessibilityNodeInfo> q=new ArrayDeque<>();q.add(root);List<N> frontier=new ArrayList<>(),rest=new ArrayList<>();int seen=0;while(!q.isEmpty()&&seen++<512){AccessibilityNodeInfo n=q.removeFirst();for(int i=0;i<n.getChildCount()&&q.size()<512;i++){AccessibilityNodeInfo c=n.getChild(i);if(c!=null)q.addLast(c);}if(n.isVisibleToUser()){N row=N.from(n);if(row.decision())frontier.add(row);else rest.add(row);}else n.recycle();}while(!q.isEmpty())q.removeFirst().recycle();List<N> all=new ArrayList<>(Math.min(256,frontier.size()+rest.size()));for(N n:frontier){if(all.size()==256){n.recycle();continue;}all.add(n);}int front=all.size();for(N n:rest){if(all.size()==256){n.recycle();continue;}all.add(n);}for(int i=0;i<all.size();i++)all.get(i).id=i;return new Scene(generation,shorten(pkg,255),all,front);}

    static final class Scene {
        final long generation;final String pkg;final List<N> nodes;final int frontier;volatile int exposed;
        Scene(long generation,String pkg,List<N> nodes,int frontier){this.generation=generation;this.pkg=pkg;this.nodes=nodes;this.frontier=frontier;}
        N ref(int id){return id>=0&&id<frontier&&id<exposed?nodes.get(id):null;}
        boolean label(String text){String find=text.toLowerCase(Locale.ROOT);for(N n:nodes)if(n.label.toLowerCase(Locale.ROOT).contains(find))return true;return false;}
        void recycle(){for(N n:nodes)n.recycle();}
    }
    static final class N {
        int id;final String label;final int role,flags;final AccessibilityNodeInfo node;
        N(String label,int role,int flags,AccessibilityNodeInfo node){this.label=label;this.role=role;this.flags=flags;this.node=node;}
        static N from(AccessibilityNodeInfo n){CharSequence t=n.getText(),d=n.getContentDescription();String r=n.getViewIdResourceName();String label=shorten(t!=null?t.toString():d!=null?d.toString():r!=null?r:"",1024);int flags=(n.isClickable()?1:0)|(n.isEnabled()?2:0)|(n.isChecked()?4:0)|(n.isScrollable()?8:0);String c=n.getClassName()==null?"":n.getClassName().toString();int role=n.isEditable()?'i':c.contains("Button")?'b':n.isCheckable()||c.contains("Switch")?'c':n.isScrollable()?'s':n.isClickable()?'m':!label.isEmpty()?'t':'u';return new N(label,role,flags,n);}
        boolean decision(){return !label.isEmpty()||(flags&9)!=0||node.isEditable()||node.isCheckable();}
        JSONArray wire(){return new JSONArray().put(id).put(label).put(role).put(flags);}
        void recycle(){node.recycle();}
    }
    private static String shorten(String s,int max){int bytes=0,end=0;while(end<s.length()){int cp=s.codePointAt(end),n=new String(Character.toChars(cp)).getBytes(java.nio.charset.StandardCharsets.UTF_8).length;if(bytes+n>max)break;bytes+=n;end+=Character.charCount(cp);}return end==s.length()?s:s.substring(0,end);}
    private static int escaped(String s){int bytes=0;for(int i=0;i<s.length();){int cp=s.codePointAt(i);bytes+=cp<32?6:cp=='"'||cp=='\\'?2:new String(Character.toChars(cp)).getBytes(java.nio.charset.StandardCharsets.UTF_8).length;i+=Character.charCount(cp);}return bytes;}
}
