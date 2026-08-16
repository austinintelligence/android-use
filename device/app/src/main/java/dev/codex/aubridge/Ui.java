package dev.codex.aubridge;

import android.accessibilityservice.AccessibilityService;
import android.graphics.Rect;
import android.os.Build;
import android.view.accessibility.AccessibilityEvent;
import android.view.accessibility.AccessibilityNodeInfo;
import android.os.SystemClock;
import org.json.JSONArray;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.List;
import java.util.Locale;

public final class Ui extends AccessibilityService {
    private static volatile Ui instance;
    private final Object guard=new Object();
    private long generation=1;
    private long lastInvalidation;
    private Scene scene;

    static Ui get(){return instance;}
    Object guard(){return guard;}
    @Override protected void onServiceConnected(){instance=this;invalidate();}
    @Override public void onAccessibilityEvent(AccessibilityEvent event){if(shouldInvalidate(event.getEventType(),event.getContentChangeTypes()))invalidate();}
    @Override public void onInterrupt(){}
    @Override public void onDestroy(){if(instance==this)instance=null;clear();super.onDestroy();}
    long generation(){synchronized(guard){return generation;}}
    private void invalidate(){synchronized(guard){long now=SystemClock.uptimeMillis();if(now-lastInvalidation<40&&scene==null)return;lastInvalidation=now;generation++;clearLocked();}}
    private void clear(){synchronized(guard){clearLocked();}}
    private void clearLocked(){if(scene!=null){scene.recycle();scene=null;}}

    static boolean shouldInvalidate(int type,int changes){
        if(type==AccessibilityEvent.TYPE_ANNOUNCEMENT||type==AccessibilityEvent.TYPE_NOTIFICATION_STATE_CHANGED)return false;
        if(type==AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED){
            int relevant=AccessibilityEvent.CONTENT_CHANGE_TYPE_TEXT|AccessibilityEvent.CONTENT_CHANGE_TYPE_CONTENT_DESCRIPTION|AccessibilityEvent.CONTENT_CHANGE_TYPE_STATE_DESCRIPTION|AccessibilityEvent.CONTENT_CHANGE_TYPE_SUBTREE;
            return changes==AccessibilityEvent.CONTENT_CHANGE_TYPE_UNDEFINED||(changes&relevant)!=0;
        }
        return type==AccessibilityEvent.TYPE_WINDOW_STATE_CHANGED||type==AccessibilityEvent.TYPE_VIEW_TEXT_CHANGED||type==AccessibilityEvent.TYPE_VIEW_CLICKED||type==AccessibilityEvent.TYPE_VIEW_SELECTED||type==AccessibilityEvent.TYPE_VIEW_FOCUSED||type==AccessibilityEvent.TYPE_VIEW_SCROLLED||type==AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUSED||type==AccessibilityEvent.TYPE_VIEW_CONTEXT_CLICKED;
    }

    Scene snapshot(){synchronized(guard){if(scene==null)scene=build();return scene;}}
    JSONArray observe(long seq,String base,int detail)throws Bridge.Limit {Scene s=snapshot();if(base!=null&&base.equals(Long.toString(s.generation)))return new JSONArray().put(seq).put(0).put(s.generation);int limit=detail==0?Math.min(s.frontier,64):Math.min(s.nodes.size(),256),bytes=0;JSONArray rows=new JSONArray();for(int i=0;i<limit;i++){N n=s.nodes.get(i);int cost=escaped(n.label)+24;if(detail==0&&rows.length()>0&&bytes+cost>3500)break;rows.put(n.wire());bytes+=cost;}s.exposed=Math.min(s.frontier,rows.length());return new JSONArray().put(seq).put(0).put(s.generation).put(s.pkg).put(rows);}

    JSONArray semantic(String focus)throws Bridge.Limit {
        Scene s=snapshot();
        List<SemanticCompiler.Row> rows=compileSemantic(s);
        String needle=normalize(focus);
        JSONArray out=new JSONArray();
        int bytes=0;
        for(SemanticCompiler.Row row:rows){
            String label=shorten(row.label,512),value=shorten(row.value,512),kind=shorten(row.kind,64),state=shorten(row.state,64);
            if(!needle.isEmpty()&&!normalize(label+" "+value+" "+state+" "+kind).contains(needle))continue;
            int cost=escaped(label)+escaped(value)+escaped(state)+escaped(kind)+48;
            if(out.length()>0&&bytes+cost>18_000)break;
            out.put(new JSONArray().put(label).put(value).put(kind).put(state).put(row.enabled).put(row.selected));
            bytes+=cost;
        }
        return out;
    }

    JSONArray resolve(String label)throws Bridge.Limit {
        Scene s=snapshot();
        s.exposed=Math.min(s.frontier,s.nodes.size());
        String needle=normalize(label);
        List<N> matches=new ArrayList<>();
        for(N n:s.nodes)if(!n.semanticLabel.isEmpty()&&normalize(n.semanticLabel).equals(needle))matches.add(n);
        if(matches.isEmpty())for(N n:s.nodes)if(!n.semanticLabel.isEmpty()&&normalize(n.semanticLabel).contains(needle))matches.add(n);
        JSONArray out=new JSONArray();
        for(N match:matches){
            N owner=owner(s,match);
            if(owner!=null&&!containsRef(out,owner.id))out.put(owner.id);
        }
        return out;
    }

    private static N owner(Scene s,N match){
        if(match.interactive()&&!match.node.isScrollable())return match;
        N fallback=null;
        for(N candidate:s.nodes){
            if(candidate.sourceId==match.parentSourceId&&candidate.interactive()){
                if(!candidate.node.isScrollable())return candidate;
                fallback=candidate;
            }
        }
        for(N candidate:s.nodes){
            if(candidate.parentSourceId==match.parentSourceId&&candidate.interactive()){
                if(!candidate.node.isScrollable())return candidate;
                fallback=candidate;
            }
        }
        N spatial=spatialOwner(s,match);
        return spatial!=null?spatial:(fallback==null?match:fallback);
    }

    private static N spatialOwner(Scene s,N match){
        int cx=(match.left+match.right)/2,cy=(match.top+match.bottom)/2;
        N best=null;long bestArea=Long.MAX_VALUE;long bestDistance=Long.MAX_VALUE;
        for(N candidate:s.nodes){
            if(!candidate.node.isClickable()||candidate.node.isScrollable())continue;
            boolean contains=cx>=candidate.left&&cx<=candidate.right&&cy>=candidate.top&&cy<=candidate.bottom;
            long area=Math.max(1L,(long)(candidate.right-candidate.left)*(candidate.bottom-candidate.top));
            long dx=cx-(candidate.left+candidate.right)/2L,dy=cy-(candidate.top+candidate.bottom)/2L;
            long distance=dx*dx+dy*dy;
            if(contains&&(best==null||area<bestArea)){best=candidate;bestArea=area;bestDistance=distance;}
            else if(best==null&&distance<bestDistance){best=candidate;bestDistance=distance;}
        }
        return best;
    }

    private static boolean containsRef(JSONArray values,int ref){for(int i=0;i<values.length();i++)if(values.optInt(i,-1)==ref)return true;return false;}

    private static List<SemanticCompiler.Row> compileSemantic(Scene s){
        List<SemanticCompiler.Item> input=new ArrayList<>();
        for(N n:s.nodes){
            String c=className(n),lower=c.toLowerCase(Locale.ROOT);
            input.add(new SemanticCompiler.Item(n.sourceId>=0?n.sourceId:n.id,n.parentSourceId,n.semanticLabel,n.hint,c,n.node.isClickable(),n.node.isEnabled(),n.node.isCheckable(),n.node.isChecked(),n.node.isEditable(),n.password,n.hasText,n.node.isScrollable(),n.node.isSelected(),isHeading(n),lower.contains("button"),lower.contains("link"),lower.contains("radio"),lower.contains("checkbox"),lower.contains("seekbar")||lower.contains("ratingbar"),true,false,n.left,n.top,n.right,n.bottom));
        }
        return SemanticCompiler.compile(input);
    }

    private static boolean isHeading(N n){return Build.VERSION.SDK_INT>=28&&n.node.isHeading();}
    private static String className(N n){return n.node.getClassName()==null?"":n.node.getClassName().toString();}
    private static String normalize(String value){return value==null?"":value.trim().toLowerCase(Locale.ROOT).replaceAll("\\s+"," ");}

    private Scene build(){
        AccessibilityNodeInfo root=getRootInActiveWindow();
        if(root==null)return new Scene(generation,"",new ArrayList<>(),0);
        String pkg=root.getPackageName()==null?"":root.getPackageName().toString();
        ArrayDeque<Visit> q=new ArrayDeque<>();
        long nextSource=1;
        q.add(new Visit(root,nextSource++,-1));
        List<N> frontier=new ArrayList<>(),rest=new ArrayList<>();
        int seen=0;
        while(!q.isEmpty()&&seen++<512){
            Visit visit=q.removeFirst();
            AccessibilityNodeInfo n=visit.node;
            for(int i=0;i<n.getChildCount()&&q.size()<512;i++){
                AccessibilityNodeInfo c=n.getChild(i);
                if(c!=null)q.addLast(new Visit(c,nextSource++,visit.sourceId));
            }
            if(n.isVisibleToUser()){
                N row=N.from(n,visit.sourceId,visit.parentSourceId);
                if(row.decision())frontier.add(row);else rest.add(row);
            }else n.recycle();
        }
        while(!q.isEmpty())q.removeFirst().recycle();
        List<N> all=new ArrayList<>(Math.min(256,frontier.size()+rest.size()));
        for(N n:frontier){if(all.size()==256){n.recycle();continue;}all.add(n);}
        int front=all.size();
        for(N n:rest){if(all.size()==256){n.recycle();continue;}all.add(n);}
        for(int i=0;i<all.size();i++)all.get(i).id=i;
        return new Scene(generation,shorten(pkg,255),all,front);
    }

    static final class Visit {
        final AccessibilityNodeInfo node;final long sourceId,parentSourceId;
        Visit(AccessibilityNodeInfo node,long sourceId,long parentSourceId){this.node=node;this.sourceId=sourceId;this.parentSourceId=parentSourceId;}
        void recycle(){node.recycle();}
    }

    static final class Scene {
        final long generation;final String pkg;final List<N> nodes;final int frontier;volatile int exposed;
        Scene(long generation,String pkg,List<N> nodes,int frontier){this.generation=generation;this.pkg=pkg;this.nodes=nodes;this.frontier=frontier;}
        N ref(int id){return id>=0&&id<frontier&&id<exposed?nodes.get(id):null;}
        boolean label(String text){String find=text.toLowerCase(Locale.ROOT);for(N n:nodes)if(n.label.toLowerCase(Locale.ROOT).contains(find))return true;return false;}
        void recycle(){for(N n:nodes)n.recycle();}
    }
    static final class N {
        int id;final String label;final int role,flags;final AccessibilityNodeInfo node;final int left,top,right,bottom;
        final long sourceId,parentSourceId;final String semanticLabel,hint;final boolean password,hasText;
        N(String label,int role,int flags,AccessibilityNodeInfo node,long sourceId,long parentSourceId,String semanticLabel,String hint,boolean password,boolean hasText,int left,int top,int right,int bottom){this.label=label;this.role=role;this.flags=flags;this.node=node;this.sourceId=sourceId;this.parentSourceId=parentSourceId;this.semanticLabel=semanticLabel;this.hint=hint;this.password=password;this.hasText=hasText;this.left=left;this.top=top;this.right=right;this.bottom=bottom;}
        static N from(AccessibilityNodeInfo n,long source,long parent){CharSequence t=n.getText(),d=n.getContentDescription(),h=n.getHintText();String text=nonEmpty(t),desc=nonEmpty(d),hint=nonEmpty(h),r=n.getViewIdResourceName();boolean password=n.isPassword(),hasText=!text.isEmpty();String semantic=shorten(password?"":(!text.isEmpty()?text:desc),1024);String label=shorten(!semantic.isEmpty()?semantic:r!=null?r:"",1024);Rect bounds=new Rect();n.getBoundsInScreen(bounds);int flags=(n.isClickable()?1:0)|(n.isEnabled()?2:0)|(n.isChecked()?4:0)|(n.isScrollable()?8:0);String c=n.getClassName()==null?"":n.getClassName().toString();int role=n.isEditable()?'i':c.contains("Button")?'b':n.isCheckable()||c.contains("Switch")?'c':n.isScrollable()?'s':!label.isEmpty()?'t':n.isClickable()?'m':'u';return new N(label,role,flags,n,source,parent,semantic,hint,password,hasText,bounds.left,bounds.top,bounds.right,bounds.bottom);}
        private static String nonEmpty(CharSequence value){return value==null?"":value.toString().trim();}
        boolean decision(){return !label.isEmpty()||(flags&9)!=0||node.isEditable()||node.isCheckable();}
        boolean interactive(){return node.isClickable()||node.isCheckable()||node.isEditable()||node.isScrollable();}
        JSONArray wire(){return new JSONArray().put(id).put(label).put(role).put(flags);}
        void recycle(){node.recycle();}
    }
    private static int utf8(int cp){return cp<0x80?1:cp<0x800?2:cp<0xD800||cp>0xDFFF&&cp<0x10000?3:cp<0x10000?1:4;}
    private static String shorten(String s,int max){int bytes=0,end=0;while(end<s.length()){int cp=s.codePointAt(end),n=utf8(cp);if(bytes+n>max)break;bytes+=n;end+=Character.charCount(cp);}return end==s.length()?s:s.substring(0,end);}
    private static int escaped(String s){int bytes=0;for(int i=0;i<s.length();){int cp=s.codePointAt(i);bytes+=cp<32?6:cp=='"'||cp=='\\'?2:utf8(cp);i+=Character.charCount(cp);}return bytes;}
}
