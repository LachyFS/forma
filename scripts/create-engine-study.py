#!/usr/bin/env python3
"""Generate an original illustrative V8 mesh study in Forma's native format.

Uses only Python's standard library. This is modelling artwork, not a dimensioned
or mechanically validated engine. Open docs/scenes/v8-engine.forma in Forma.
"""
import argparse
import json
import math
from pathlib import Path

PI = math.pi
objects, meshes, materials = [], [], []
serial = 2

def uid():
    global serial
    serial += 1
    return serial

def material(name, rgb, metallic=.7, roughness=.3):
    ident = uid()
    materials.append({'id': ident, 'name': name, 'material': {
        'base_color': rgb, 'metallic': metallic, 'roughness': roughness,
        'emission': [0, 0, 0]}})
    return ident

aluminum = material('Machined aluminum', [.48,.52,.56], .8, .28)
steel = material('Brushed steel', [.24,.29,.32], .85, .25)
polished = material('Polished aluminum', [.7,.74,.76], .95, .18)
teal = material('Petrol enamel', [.035,.22,.18], .45, .29)
rubber = material('Graphite rubber', [.018,.023,.028], .05, .72)
bronze = material('Heat-tinted titanium', [.39,.27,.16], .8, .3)
black = material('Cast graphite', [.065,.078,.084], .6, .45)

def add(name, mesh, mat, center=(0,0,0), rotate=(0,0,0)):
    mid, oid = uid(), uid()
    vertices, faces = mesh
    meshes.append({'id':mid, 'name':name, 'mesh':{'positions':[[round(v,5) for v in p] for p in vertices], 'faces':faces}})
    objects.append({'id':oid,'name':name,'transform':{'translation':center,'rotation':rotate,'scale':[1,1,1]},'data':{'type':'mesh','mesh':mid,'materials':[mat]},'visible':True,'selectable':True,'collections':[1]})

def box(w,h,d, bevel=.05):
    b=min(bevel,w/6,h/6,d/6)
    vertices=[]
    for y, inset in [(-h/2,b),(-h/2+b,0),(h/2-b,0),(h/2,b)]:
        for cx,cz,start in [(1,1,0),(-1,1,PI/2),(-1,-1,PI),(1,-1,3*PI/2)]:
            for i in range(5):
                a=start+i*PI/8
                vertices.append((cx*(w/2-b-inset)+b*math.cos(a),y,cz*(d/2-b-inset)+b*math.sin(a)))
    n=20
    faces=[[j for j in reversed(range(n))]]
    for k in range(3):
        for j in range(n):faces.append([k*n+j,k*n+(j+1)%n,(k+1)*n+(j+1)%n,(k+1)*n+j])
    faces.append([3*n+j for j in range(n)])
    # XZ perimeter runs counter-clockwise from above: flip for outward winding.
    return vertices,[list(reversed(f)) for f in faces]

def lathe(profile, segments=48):
    vertices, rings, faces = [], [], []
    for r,y in profile:
        if r == 0:
            rings.append([len(vertices)])
            vertices.append((0,y,0))
        else:
            rings.append(list(range(len(vertices),len(vertices)+segments)))
            vertices.extend((r*math.cos(2*PI*j/segments),y,r*math.sin(2*PI*j/segments)) for j in range(segments))
    for a,b in zip(rings,rings[1:]):
        for j in range(segments):
            if len(a)==1: faces.append([a[0],b[j],b[(j+1)%segments]])
            elif len(b)==1: faces.append([a[j],b[0],a[(j+1)%segments]])
            else: faces.append([a[j],b[j],b[(j+1)%segments],a[(j+1)%segments]])
    return vertices,faces

def cylinder(r,h,segments=40):
    b=min(.025,h*.2)
    return lathe([(0,-h/2),(r*.96,-h/2),(r,-h/2+b),(r,h/2-b),(r*.96,h/2),(0,h/2)],segments)

def ring(r,inner,h,segments=48):
    return lathe([(inner,-h/2),(r,-h/2),(r,h/2),(inner,h/2),(inner,-h/2)],segments)

def unit(v):
    m=math.sqrt(sum(c*c for c in v));return tuple(c/m for c in v)
def cross(a,b):return (a[1]*b[2]-a[2]*b[1],a[2]*b[0]-a[0]*b[2],a[0]*b[1]-a[1]*b[0])
def pipe(points,r,segments=16):
    path=[]
    pts=[points[0]]+points+[points[-1]]
    for k in range(1,len(pts)-2):
        p0,p1,p2,p3=pts[k-1:k+3]
        for j in range(9):
            t=j/9
            path.append(tuple(.5*((2*p1[a])+(-p0[a]+p2[a])*t+(2*p0[a]-5*p1[a]+4*p2[a]-p3[a])*t*t+(-p0[a]+3*p1[a]-3*p2[a]+p3[a])*t*t*t) for a in range(3)))
    path.append(points[-1]);v=[];f=[];previous_normal=None
    for k,p in enumerate(path):
        before=path[max(k-1,0)];after=path[min(k+1,len(path)-1)]
        tangent=unit(tuple(after[a]-before[a] for a in range(3)))
        normal=unit(cross(tangent,(0,0,1) if abs(tangent[2])<.9 else (0,1,0))) if previous_normal is None else unit(tuple(previous_normal[a]-sum(previous_normal[c]*tangent[c] for c in range(3))*tangent[a] for a in range(3)))
        previous_normal=normal
        binormal=cross(tangent,normal)
        for j in range(segments):
            a=j*2*PI/segments
            v.append(tuple(p[c]+r*(normal[c]*math.cos(a)+binormal[c]*math.sin(a)) for c in range(3)))
        if k:
            for j in range(segments):
                a,b,c,d=(k-1)*segments+j,(k-1)*segments+(j+1)%segments,k*segments+(j+1)%segments,k*segments+j
                f.extend([[a,b,c],[a,c,d]])
    return v,f

# Main castings and sump.
add('Engine block / aluminum',box(2.25,1.05,3.5,.12),aluminum,(0,.88,0))
add('Sump flange',box(2.38,.14,3.64),steel,(0,.32,0))
add('Oil sump',box(1.95,.5,3.12,.12),black,(0,.04,0))
for x in [-.78,-.52,-.26,0,.26,.52,.78]:
    add('Sump cooling rib',box(.065,.08,2.82,.012),aluminum,(x,-.23,0))
for z in [-1.47,-1.1,-.73,-.36,0,.36,.73,1.1,1.47]:
    for x in [-1.13,1.13]:add('Sump fastener',cylinder(.062,.09,6),polished,(x,.39,z))

# Two cylinder banks, head-gasket seams, ribbed enamel cam covers.
for side in [-1,1]:
    angle=-side*.58
    add('Cylinder head / '+('left' if side<0 else 'right'),box(1.1,.66,3.55,.09),aluminum,(side*.95,1.48,0),(0,0,angle))
    add('Head gasket',box(1.16,.045,3.57,.025),rubber,(side*1.13,1.76,0),(0,0,angle))
    add('Cam cover / petrol enamel',box(1.08,.26,3.48,.105),teal,(side*1.19,1.9,0),(0,0,angle))
    for offset in [-.36,-.24,-.12,0,.12,.24,.36]:
        x=side*1.19+offset*math.cos(angle)-.145*math.sin(angle)
        y=1.9+offset*math.sin(angle)+.145*math.cos(angle)
        add('Cam cover machined rib',box(.027,.024,2.85,.007),polished,(x,y,0),(0,0,angle))
    for z in [-1.47,-.5,.5,1.47]:
        for offset in [-.44,.44]:
            x=side*1.19+offset*math.cos(angle)-.14*math.sin(angle)
            y=1.9+offset*math.sin(angle)+.14*math.cos(angle)
            add('Cover hex bolt',cylinder(.068,.08,6),polished,(x,y,z),(0,0,angle))
    # Cast ribs and freeze plugs visible between the exhaust runners.
    for z in [-1.23,-.41,.41,1.23]:
        add('Block freeze plug',cylinder(.16,.04),steel,(side*1.145,.93,z),(0,0,PI/2))
        add('Exhaust flange',box(.14,.38,.52,.05),steel,(side*1.59,1.37,z))
        for dz in [-.19,.19]:add('Exhaust flange stud',cylinder(.057,.09,6),polished,(side*1.68,1.42,z+dz),(0,0,PI/2))
        # Individual curved four-into-one header pipes.
        endz=-1.6 + (z+1.23)*.14
        add('Tubular exhaust runner',pipe([(side*1.68,1.38,z),(side*2.03,1.32,z+.03),(side*2.3,.81,z-.07),(side*2.32,.3,endz),(side*2.24,.2,-1.88)],.13,20),bronze)
        add('Exhaust weld bead',ring(.14,.122,.035,32),polished,(side*1.77,1.38,z),(0,0,PI/2))
    add('Exhaust collector',cylinder(.29,.9),steel,(side*2.24,.2,-2.16),(PI/2,0,0))
    add('Collector flange',ring(.36,.24,.1),aluminum,(side*2.24,.2,-2.64),(PI/2,0,0))

# Open velocity stacks: eight flared trumpets with dark bores and separate clamps.
for side in [-1,1]:
    for z in [-1.2,-.4,.4,1.2]:
        add('Intake runner',pipe([(side*.77,1.72,z),(side*.65,2.0,z),(side*.48,2.25,z)],.19,24),aluminum)
        add('Throttle body',cylinder(.235,.34),black,(side*.48,2.22,z))
        add('Throttle clamp',ring(.255,.21,.065),steel,(side*.48,2.39,z))
        profile=[(.19,0),(.19,.34),(.20,.48),(.25,.59),(.32,.65),(.33,.69),(.305,.71),(.28,.675),(.225,.61),(.177,.49),(.167,.34),(.167,0),(.19,0)]
        add('Velocity stack',lathe(profile,64),polished,(side*.48,2.41,z))
        add('Intake shadow',cylinder(.16,.025),rubber,(side*.48,2.48,z))
    add('Fuel rail',cylinder(.065,2.95),bronze,(side*.84,2.28,0),(PI/2,0,0))
    for z in [-1.2,-.4,.4,1.2]:
        add('Injector connector',box(.14,.18,.18,.025),black,(side*.88,2.17,z))
        add('Rail fitting',cylinder(.09,.1,6),polished,(side*.84,2.28,z),(PI/2,0,0))
add('Throttle linkage',cylinder(.04,3.2),steel,(0,2.21,0),(PI/2,0,0))
for z in [-1.2,-.4,.4,1.2]:add('Throttle bridge',box(1.03,.06,.05,.01),steel,(0,2.2,z))

# Front timing cover, pulleys, alternator and serpentine belt.
add('Timing housing',box(1.65,1.35,.22,.15),aluminum,(0,1.02,1.88))
for x in [-.7,.7]:
    for y in [.5,.83,1.16,1.52]:add('Timing cover bolt',cylinder(.06,.08,6),polished,(x,y,2.025),(PI/2,0,0))

def pulley(name,x,y,z,r):
    add(name+' / rim',cylinder(r,.21,64),black,(x,y,z),(PI/2,0,0))
    add(name+' / face',ring(r*.88,r*.42,.055,64),aluminum,(x,y,z+.13),(PI/2,0,0))
    add(name+' / hub',cylinder(r*.23,.16,32),steel,(x,y,z+.14),(PI/2,0,0))
    add(name+' / hex',cylinder(r*.11,.08,6),polished,(x,y,z+.24),(PI/2,0,0))
    for a in range(6):
        t=a*PI/3
        add(name+' / spoke',box(r*.16,.07,r*.52,.015),steel,(x+math.sin(t)*r*.46,y+math.cos(t)*r*.46,z+.145),(PI/2,t,0))
    for dz in [-.08,-.03,.025,.08]:add(name+' / groove',ring(r*1.015,r*.965,.018,64),steel,(x,y,z+dz),(PI/2,0,0))
pulley('Crank pulley',0,.65,2.19,.51)
pulley('Water pump',0,1.62,2.21,.34)
pulley('Alternator pulley',-1.29,1.28,2.21,.29)
pulley('Idler',.94,1.31,2.21,.24)
add('Alternator housing',cylinder(.42,.6),aluminum,(-1.29,1.28,1.74),(PI/2,0,0))
for a in range(20):
    t=a*PI/10
    add('Alternator cooling slot',box(.07,.1,.44,.015),black,(-1.29+math.sin(t)*.405,1.28+math.cos(t)*.405,1.74),(0,0,-t))
# Two visible belt lengths run between the outer pulleys.
for a,b in [((-1.57,1.32,2.18),(-.42,.29,2.18)),((.4,.27,2.18),(1.17,1.22,2.18)),((1.14,1.48,2.18),(.15,1.95,2.18)),((-.22,1.88,2.18),(-1.42,1.52,2.18))]:
    add('Drive belt',pipe([a,b],.045,8),rubber)
add('Coolant neck',pipe([(.78,1.84,1.56),(.94,2.11,1.85),(1.44,2.1,2.02)],.145,24),aluminum)
add('Coolant hose',pipe([(1.44,2.1,2.02),(1.78,2.12,1.95),(1.9,1.89,1.1)],.155,24),rubber)
add('Oil filler cap',cylinder(.2,.14,32),black,(1.09,2.16,-1.18),(0,0,-.58))
for side in [-1,1]:
    add('Engine mount',box(.65,.14,.58,.045),steel,(side*1.4,.57,.6))
    add('Engine mount bush',cylinder(.14,.16),rubber,(side*1.64,.66,.6))

scene={'objects':objects,'meshes':meshes,'materials':materials,'collections':[{'id':1,'name':'V8 engine study','parent':None,'visible':True}],'root_collection':1,'camera':{'target':[0,1.25,0],'yaw':.68,'pitch':.4,'distance':11.7,'fov_y':.67,'orthographic':False},'world':{'color':[.48,.58,.72],'strength':.35},'render':{'exposure':0,'max_samples':128,'max_bounces':8},'next_id':serial+1}
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--exploded', action='store_true', help='Separate the assemblies for an editing study')
args = parser.parse_args()
if args.exploded:
    mesh_by_id = {m['id']: m['mesh'] for m in meshes}
    for obj in objects:
        name = obj['name']
        origin = obj['transform']['translation']
        vertices = mesh_by_id[obj['data']['mesh']]['positions']
        center_x = origin[0] + sum(p[0] for p in vertices) / len(vertices)
        side = -1 if center_x < 0 else 1
        offset = [0,0,0]
        if name.startswith(('Cam cover', 'Cover hex', 'Oil filler')):
            offset = [side*.8, .9, 0]
        elif name.startswith('Cylinder head'):
            offset = [side*.25, .24, 0]
        elif name == 'Head gasket':
            offset = [side*.47, .54, 0]
        elif name.startswith(('Velocity stack', 'Intake', 'Throttle', 'Fuel rail', 'Injector', 'Rail fitting')):
            offset = [0, .85, 0]
        elif name.startswith(('Tubular exhaust', 'Exhaust', 'Collector')):
            offset = [side*.95, 0, 0]
        elif name.startswith(('Crank pulley', 'Water pump', 'Alternator', 'Idler', 'Drive belt')):
            offset = [0, 0, .8]
        elif name.startswith(('Timing housing', 'Timing cover')):
            offset = [0, 0, .35]
        elif name in ['Oil sump', 'Sump cooling rib']:
            offset = [0, -.65, 0]
        elif name == 'Sump flange':
            offset = [0, -.2, 0]
        obj['transform']['translation'] = [round(origin[i]+offset[i],5) for i in range(3)]
    objects.sort(key=lambda obj: not (obj['name'] == 'Cam cover / petrol enamel' and obj['transform']['translation'][0] > 0))
    scene['camera'].update(target=[0,1.45,0], distance=13.5, pitch=.38)
    scene['collections'][0]['name'] = 'V8 exploded study'
filename = 'v8-engine-exploded.forma' if args.exploded else 'v8-engine.forma'
path=Path(__file__).resolve().parents[1]/'docs/scenes'/filename
path.parent.mkdir(parents=True,exist_ok=True)
path.write_text(json.dumps({'format':'forma','version':3,'scene':scene},separators=(',',':'))+'\n')
print(f'{path}: {len(objects)} objects, {sum(len(m["mesh"]["positions"]) for m in meshes):,} vertices, {path.stat().st_size:,} bytes')
