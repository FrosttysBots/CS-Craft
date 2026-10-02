// Observe the installed client's baked model and animation, without decompiling it.
// Output is a local cache beside world saves; no game assets are distributed.
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import com.google.gson.Gson;

public class ExportDragon {
    static final List<Object> nodes = new ArrayList<>();
    static final List<Map<String,Object>> parts = new ArrayList<>();
    static Object field(Object o, String n) throws Exception {
        Field f = o.getClass().getDeclaredField(n); f.setAccessible(true); return f.get(o);
    }
    static float number(Object o, String n) throws Exception { return ((Number)field(o,n)).floatValue(); }
    static void visit(Object node, int parent, String name) throws Exception {
        int index = nodes.size(); nodes.add(node);
        var part = new LinkedHashMap<String,Object>();
        part.put("parent",parent); part.put("name",name);
        var quads = new ArrayList<Object>();
        for (Object cube : (List<?>)field(node,"cubes")) {
            for (Object polygon : (Object[])field(cube,"polygons")) {
                var quad = new ArrayList<Object>();
                for (Object v : (Object[])polygon.getClass().getMethod("vertices").invoke(polygon)) {
                    var row = new float[5]; String[] names={"x","y","z","u","v"};
                    for (int i=0;i<5;i++) row[i]=((Number)v.getClass().getMethod(names[i]).invoke(v)).floatValue();
                    quad.add(row);
                }
                quads.add(quad);
            }
        }
        part.put("quads",quads); parts.add(part);
        var children = (Map<?,?>)field(node,"children");
        var names = new ArrayList<String>(); for(Object key:children.keySet()) names.add((String)key);
        Collections.sort(names);
        for(String key:names) visit(children.get(key),index,key);
    }
    public static void main(String[] args) throws Exception {
        var modelClass=Class.forName("net.minecraft.client.model.monster.dragon.EnderDragonModel");
        var layer=modelClass.getMethod("createBodyLayer").invoke(null);
        var root=layer.getClass().getMethod("bakeRoot").invoke(layer);
        var model=modelClass.getConstructor(root.getClass()).newInstance(root);
        visit(root,-1,"root");
        var stateClass=Class.forName("net.minecraft.client.renderer.entity.state.EnderDragonRenderState");
        var clips = new ArrayList<Object>();
        for(boolean sitting:new boolean[]{false,true}) {
            var frames = new ArrayList<Object>();
            for(int frame=0;frame<64;frame++) {
                for(Object node:nodes) node.getClass().getMethod("resetPose").invoke(node);
                var state=stateClass.getConstructor().newInstance();
                stateClass.getField("flapTime").setFloat(state,frame/64f);
                stateClass.getField("partialTicks").setFloat(state,1f);
                stateClass.getField("isSitting").setBoolean(state,sitting);
                var history=stateClass.getField("flightHistory").get(state);
                for(int i=0;i<128;i++) history.getClass().getMethod("record",double.class,float.class).invoke(history,80.,0f);
                modelClass.getMethod("setupAnim",stateClass).invoke(model,state);
                var poses = new ArrayList<Object>();
                for(Object node:nodes) {
                    var pose = new float[9]; String[] fields={"x","y","z","xRot","yRot","zRot","xScale","yScale","zScale"};
                    for(int i=0;i<9;i++) pose[i]=number(node,fields[i]);
                    poses.add(pose);
                }
                frames.add(poses);
            }
            clips.add(frames);
        }
        Files.writeString(Path.of(args[0]),new Gson().toJson(Map.of("parts",parts,"clips",clips)));
        System.out.println("Exported "+parts.size()+" dragon parts; 64 frames per flight/perch clip.");
    }
}
