var farm_p_a = require('./a');
var farm_p_b = require('./b');
var farm_p = await Promise.all([farm_p_a, farm_p_b]);
var a = farm_p[0].default;
var b = farm_p[1].default;
console.log(a, b);