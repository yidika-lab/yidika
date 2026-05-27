ce n'est pas const x: int = 5 du yidika ça aucune variable commence par un mot clé dans yidika une const peut  être x:const = 5 ou x:int = 5 as const 

il faut comprendre

yidika est une tire inspiration de kotlin , Typescript , Python et rust

- dans yidika doit avoir aussi des fonction anonyme
fn 
() =>

des callback fonction dans yidika 
une callback function peut avoir son propre body

comme 

fn doIt(callback:Function){}

on peut passer la callback comme js 
doIT((par)=>{
    body
})

mais aussi 
(cette n'est possible que s)
doIT():(par){
    body
}

sur les boucles 

- il y a la for, while , infinity, loop

dans la for on peut
itarable est tout ce qui peut boucle , comme les liste table objet etc 

- for(key:iterable){} // c'est comme le for of
- for(key in iterable) {} // c'est comme le for in 

interable peut aussi être 

- 1...10 // ça veut dire commence à 1 et arrêt si c'est 10 , donc une incrementation
- 0...10 // ça veut dire commence à 0 et arrêt si c'est 10 , donc une incrementation
- -10...0 // ça veut dire commence à -10 et arrêt si c'est 0 , donc une incrementation
- -10...-1 // ça veut dire commence à -10 et arrêt si c'est -1 , donc une incrementation
- 10...-1 // ça veut dire commence à 10 et arrêt si c'est -1 , donc une decrementation
mais aussi 

sur de character comme avec Kotlin
 - 'a'...'z' // ça veut dire commence à a et arrêt si c'est z
 - 'A'...'Z' // ça veut dire commence à A et arrêt si c'est Z
 - '0'...'9' // ça veut dire commence à 0 et arrêt si c'est 9
 - 'a'...'a' // ça veut dire commence à z et arrêt si c'est a

 - Null = 0
 - None = Empty

 dans les import 

 use json from 'json'
 use {parse, stringify} from 'json'

 use data as const from 'json:./data.json' // grace à data on peut modifier la data.json mais aussi la lecture comme un objet

 data.age = 18 

 use math from 'math'
 use {cos} from 'math'

 math.cos(90)

dans yidika le if est un statement comme dans Kotlin
 if (data.age > 18) {
    console.log('tu es majeur')
 }

permi = if(data.age > 18) {'tu es majeur'}else{'tu es mineur'}
ou 

permi = data.age > 18 ? 'tu es majeur' : 'tu es mineur'

NB : dans yidika le ; est optionnel mais recommandé


##  utilisation


```
name(<argr>)
```

NB :

la lib
 - stdlib fait partie
 - net ( use {server} from 'net')
     - http
     - https
     - http2
     - http3
     - socket
     - udp
     - tcp
     - tls
     - ssl
     - dns
     - ip
     - ipv4
     - ipv6
     - ipv6m
     - ipv6s
     - smtp
     ...
     - Server objet ( qui permet de créer les serveur les plus rapide au monde et plus simple mieux bun , rust node)

     NB: yidika doit avoir une maitrise des resources
      savoir le nombre de de coeurs du systeme, la puisance de CPU , NPU , RAM , GPU disponible ensuite adapter le le compilateur , l'interpreteur en consequence pour optimiser les performances , mais yidika doit aussi avoir ces propres resources virtuelles pour aller pour vite que jamais plus vites que RUST , GO 

      - le parallèle
      - l'asynchrone et autres doit être meilleurs que jamais

      NB : yidika doit être un langage de performance qui s'adapte à la performance du systeme qui l'execute

      - le biniaire doit être plus petit que jamais et optimisé pour la machine qui l'execute


      -