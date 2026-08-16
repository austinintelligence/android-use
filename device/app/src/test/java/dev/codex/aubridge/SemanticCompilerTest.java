package dev.codex.aubridge;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import org.junit.Test;

public final class SemanticCompilerTest {
    private static SemanticCompiler.Item text(long id,long parent,String label,boolean clickable,boolean selected){
        return new SemanticCompiler.Item(id,parent,label,"","TextView",clickable,true,false,false,false,false,false,false,selected,false,false,false,false,false,false,true,false);
    }
    private static SemanticCompiler.Item value(long id,long parent,String label){
        return text(id,parent,label,false,false);
    }
    private static SemanticCompiler.Item heading(long id,long parent,String label){
        return new SemanticCompiler.Item(id,parent,label,"","Heading",false,true,false,false,false,false,false,false,false,true,false,false,false,false,false,true,false);
    }
    private static SemanticCompiler.Item switchItem(long id,long parent,boolean checked){
        return new SemanticCompiler.Item(id,parent,"","","Toggle",true,true,true,checked,false,false,false,false,false,false,false,false,false,false,false,true,false);
    }
    private static SemanticCompiler.Item button(long id,long parent,String label,boolean enabled){
        return new SemanticCompiler.Item(id,parent,label,"","Button",true,enabled,false,false,false,false,false,false,false,false,true,false,false,false,false,true,false);
    }
    private static SemanticCompiler.Item password(long id,long parent,boolean filled){
        return new SemanticCompiler.Item(id,parent,"","Password","EditText",true,true,false,false,true,true,filled,false,false,false,false,false,false,false,false,true,false);
    }
    private static SemanticCompiler.Item boundedText(long id,long parent,String label,int left,int top,int right,int bottom){
        return new SemanticCompiler.Item(id,parent,label,"","TextView",false,true,false,false,false,false,false,false,false,false,false,false,false,false,false,true,false,left,top,right,bottom);
    }
    private static SemanticCompiler.Item boundedOwner(long id,long parent,String label,int left,int top,int right,int bottom){
        return new SemanticCompiler.Item(id,parent,label,"","Row",true,true,false,false,false,false,false,false,false,false,false,false,false,false,false,true,false,left,top,right,bottom);
    }
    private static SemanticCompiler.Item boundedSwitch(long id,long parent,boolean checked,int left,int top,int right,int bottom){
        return new SemanticCompiler.Item(id,parent,"","","Toggle",false,true,true,checked,false,false,false,false,false,false,false,false,false,false,false,true,false,left,top,right,bottom);
    }
    private static String rows(List<SemanticCompiler.Row> rows){
        StringBuilder out=new StringBuilder();
        for(SemanticCompiler.Row row:rows){
            if(out.length()>0)out.append("\n");
            out.append(row.label).append("|").append(row.value).append("|").append(row.kind).append("|").append(row.state).append("|").append(row.enabled).append("|").append(row.selected);
        }
        return out.toString();
    }

    @Test public void fusesValuesAndUsesRolesAsEvidenceNotNames(){
        List<SemanticCompiler.Item> input=Arrays.asList(
            heading(901,900,"Overview"),
            value(902,910,"Account name"), value(903,910,"Generic network"),
            text(904,911,"Flight mode",false,false), switchItem(905,911,false),
            button(906,912,"Continue",true),
            text(907,913,"Secret",false,false), password(908,913,true));
        String result=rows(SemanticCompiler.compile(input));
        assertTrue(result,result.contains("Account name|Generic network"));
        assertTrue(result,result.contains("Flight mode||switch|unchecked"));
        assertTrue(result,result.contains("Continue||button"));
        assertTrue(result,result.contains("Password|filled password|text field"));
        assertFalse(result.contains("Password value"));
    }

    @Test public void meaninglessNodesAndRenumberingDoNotChangeMeaning(){
        List<SemanticCompiler.Item> base=Arrays.asList(
            heading(1,2,"Dashboard"), value(3,4,"Metric"), value(5,4,"42%"),
            text(6,7,"Notifications",false,false), switchItem(8,7,true));
        List<SemanticCompiler.Item> mutated=Arrays.asList(
            new SemanticCompiler.Item(700,701,"","","FrameLayout",false,true,false,false,false,false,false,false,false,false,false,false,false,false,false,true,false),
            heading(1701,1702,"Dashboard"),
            new SemanticCompiler.Item(1703,1704,"","","Wrapper",false,true,false,false,false,false,false,false,false,false,false,false,false,false,false,true,false),
            value(1705,1704,"Metric"), value(1706,1704,"42%"),
            new SemanticCompiler.Item(1707,1708,"","","Wrapper",false,true,false,false,false,false,false,false,false,false,false,false,false,false,false,true,false),
            text(1709,1708,"Notifications",false,false), switchItem(1710,1708,true));
        assertEquals(rows(SemanticCompiler.compile(base)),rows(SemanticCompiler.compile(mutated)));
    }

    @Test public void modalItemsDominateBackgroundAndDuplicateLabelsRemainDistinct(){
        List<SemanticCompiler.Item> input=new ArrayList<>();
        input.add(button(1,2,"Delete",true));
        input.add(new SemanticCompiler.Item(3,4,"Delete account?","","Dialog",false,true,false,false,false,false,false,false,false,true,false,false,false,false,false,true,true));
        input.add(new SemanticCompiler.Item(5,4,"Cancel","","Button",true,true,false,false,false,false,false,false,false,false,true,false,false,false,false,true,true));
        input.add(new SemanticCompiler.Item(6,4,"Delete","","Button",true,true,false,false,false,false,false,false,false,false,true,false,false,false,false,true,true));
        List<SemanticCompiler.Row> result=SemanticCompiler.compile(input);
        assertEquals(rows(result),3,result.size());
        assertEquals("Delete account?",result.get(0).label);
        assertEquals("Delete",result.get(2).label);
        assertTrue(result.get(2).kind.equals("button"));
    }

    @Test public void geometryFusesNestedRowsAndIgnoresDecorativeLabels(){
        List<SemanticCompiler.Item> input=Arrays.asList(
            new SemanticCompiler.Item(1,2,"Overview","","Heading",false,true,false,false,false,false,false,false,false,true,false,false,false,false,false,true,false,0,0,900,40),
            boundedOwner(10,100,"",0,50,900,110),
            boundedText(11,10,"Wireless access",40,68,220,94),
            boundedSwitch(12,10,true,820,58,880,102),
            boundedOwner(20,100,"Node-7,Ready,Protected",0,112,900,174),
            boundedText(21,20,"Node-7",40,126,220,150),
            boundedText(22,20,"Ready",40,150,180,170),
            new SemanticCompiler.Item(23,20,"Options","","ImageView",false,true,false,false,false,false,false,false,false,false,false,false,false,false,false,true,false,820,120,880,168));
        String result=rows(SemanticCompiler.compile(input));
        assertTrue(result,result.contains("Wireless access||switch|checked"));
        assertTrue(result,result.contains("Node-7|Ready, Protected"));
        assertFalse(result,result.contains("Node-7,Ready,Protected"));
        assertFalse(result,result.contains("Options"));

        List<SemanticCompiler.Item> mutated=Arrays.asList(
            new SemanticCompiler.Item(7000,7001,"","","FrameLayout",false,true,false,false,false,false,false,false,false,false,false,false,false,false,false,true,false),
            boundedOwner(2100,9200,"Node-7,Ready,Protected",0,112,900,174),
            new SemanticCompiler.Item(7002,7003,"","","Wrapper",false,true,false,false,false,false,false,false,false,false,false,false,false,false,false,true,false),
            boundedText(2201,2100,"Ready",40,150,180,170),
            boundedOwner(2200,9200,"",0,50,900,110),
            boundedSwitch(2202,2200,true,820,58,880,102),
            boundedText(2203,2200,"Wireless access",40,68,220,94),
            new SemanticCompiler.Item(2103,2100,"Thumbnail","","ImageView",false,true,false,false,false,false,false,false,false,false,false,false,false,false,false,true,false,820,120,880,168),
            new SemanticCompiler.Item(2000,2001,"Overview","","Heading",false,true,false,false,false,false,false,false,false,true,false,false,false,false,false,true,false,0,0,900,40),
            boundedText(2101,2100,"Node-7",40,126,220,150));
        assertEquals(result,rows(SemanticCompiler.compile(mutated)));
    }
}
