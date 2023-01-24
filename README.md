# World Maps

An experiment in rendering [MBTiles](https://github.com/mapbox/mbtiles-spec) vector tilesets with [Mapbox GL Styles](https://docs.mapbox.com/mapbox-gl-js/style-spec/)

In order to run the program you will need to acquire a mbtiles file, you can find some generated from OpenStreetMap on [archive.org](https://archive.org/details/osm-vector-mbtiles) - only vector tiles are supported. A Mapbox GL style will also been required, some good example styles are provided by [OpenMapTiles](https://openmaptiles.org/styles/).

```
$ cargo run --release -- --style mapbox_style.json tile_data.mbtiles
```

![World Maps Demo](assets/demo.png)
