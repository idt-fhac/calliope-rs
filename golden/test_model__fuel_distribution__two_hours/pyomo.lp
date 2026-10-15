\* Source Pyomo model name=None *\

min 
objectives(min_cost_optimisation)(0):
+1.0 variables(flow_in)(a__test_conversion__coal__2005_01_01_00_00)
+2.0 variables(flow_in)(a__test_conversion__gas__2005_01_01_00_00)
+1.0 variables(flow_in)(a__test_conversion__coal__2005_01_01_01_00)
+2.0 variables(flow_in)(a__test_conversion__gas__2005_01_01_01_00)

s.t.

c_e_constraints(balance_conversion)(a__test_conversion__2005_01_01_00_00)_:
-1 variables(flow_in)(a__test_conversion__coal__2005_01_01_00_00)
-1 variables(flow_in)(a__test_conversion__gas__2005_01_01_00_00)
+1.1111111111111112 variables(flow_out)(a__test_conversion__electricity__2005_01_01_00_00)
= 0.0

c_e_constraints(balance_conversion)(a__test_conversion__2005_01_01_01_00)_:
-1 variables(flow_in)(a__test_conversion__coal__2005_01_01_01_00)
-1 variables(flow_in)(a__test_conversion__gas__2005_01_01_01_00)
+1.1111111111111112 variables(flow_out)(a__test_conversion__electricity__2005_01_01_01_00)
= 0.0

c_e_constraints(balance_conversion)(b__test_conversion__2005_01_01_00_00)_:
+1.1111111111111112 variables(flow_out)(b__test_conversion__electricity__2005_01_01_00_00)
-1 variables(flow_in)(b__test_conversion__coal__2005_01_01_00_00)
-1 variables(flow_in)(b__test_conversion__gas__2005_01_01_00_00)
= 0.0

c_e_constraints(balance_conversion)(b__test_conversion__2005_01_01_01_00)_:
+1.1111111111111112 variables(flow_out)(b__test_conversion__electricity__2005_01_01_01_00)
-1 variables(flow_in)(b__test_conversion__coal__2005_01_01_01_00)
-1 variables(flow_in)(b__test_conversion__gas__2005_01_01_01_00)
= 0.0

c_u_constraints(flow_out_max)(a__test_conversion__electricity__2005_01_01_00_00)_:
+1 variables(flow_out)(a__test_conversion__electricity__2005_01_01_00_00)
-1.0 variables(flow_cap)(a__test_conversion__electricity)
<= 0.0

c_u_constraints(flow_out_max)(a__test_conversion__electricity__2005_01_01_01_00)_:
+1 variables(flow_out)(a__test_conversion__electricity__2005_01_01_01_00)
-1.0 variables(flow_cap)(a__test_conversion__electricity)
<= 0.0

c_u_constraints(flow_out_max)(a__test_supply_coal__coal__2005_01_01_00_00)_:
+1 variables(flow_out)(a__test_supply_coal__coal__2005_01_01_00_00)
-1.0 variables(flow_cap)(a__test_supply_coal__coal)
<= 0.0

c_u_constraints(flow_out_max)(a__test_supply_coal__coal__2005_01_01_01_00)_:
-1.0 variables(flow_cap)(a__test_supply_coal__coal)
+1 variables(flow_out)(a__test_supply_coal__coal__2005_01_01_01_00)
<= 0.0

c_u_constraints(flow_out_max)(a__test_supply_gas__gas__2005_01_01_00_00)_:
+1 variables(flow_out)(a__test_supply_gas__gas__2005_01_01_00_00)
-1.0 variables(flow_cap)(a__test_supply_gas__gas)
<= 0.0

c_u_constraints(flow_out_max)(a__test_supply_gas__gas__2005_01_01_01_00)_:
-1.0 variables(flow_cap)(a__test_supply_gas__gas)
+1 variables(flow_out)(a__test_supply_gas__gas__2005_01_01_01_00)
<= 0.0

c_u_constraints(flow_out_max)(b__test_conversion__electricity__2005_01_01_00_00)_:
+1 variables(flow_out)(b__test_conversion__electricity__2005_01_01_00_00)
-1.0 variables(flow_cap)(b__test_conversion__electricity)
<= 0.0

c_u_constraints(flow_out_max)(b__test_conversion__electricity__2005_01_01_01_00)_:
+1 variables(flow_out)(b__test_conversion__electricity__2005_01_01_01_00)
-1.0 variables(flow_cap)(b__test_conversion__electricity)
<= 0.0

c_u_constraints(flow_out_max)(b__test_supply_coal__coal__2005_01_01_00_00)_:
+1 variables(flow_out)(b__test_supply_coal__coal__2005_01_01_00_00)
-1.0 variables(flow_cap)(b__test_supply_coal__coal)
<= 0.0

c_u_constraints(flow_out_max)(b__test_supply_coal__coal__2005_01_01_01_00)_:
-1.0 variables(flow_cap)(b__test_supply_coal__coal)
+1 variables(flow_out)(b__test_supply_coal__coal__2005_01_01_01_00)
<= 0.0

c_u_constraints(flow_out_max)(b__test_supply_gas__gas__2005_01_01_00_00)_:
+1 variables(flow_out)(b__test_supply_gas__gas__2005_01_01_00_00)
-1.0 variables(flow_cap)(b__test_supply_gas__gas)
<= 0.0

c_u_constraints(flow_out_max)(b__test_supply_gas__gas__2005_01_01_01_00)_:
-1.0 variables(flow_cap)(b__test_supply_gas__gas)
+1 variables(flow_out)(b__test_supply_gas__gas__2005_01_01_01_00)
<= 0.0

c_u_constraints(flow_in_max)(a__test_conversion__coal__2005_01_01_00_00)_:
+1 variables(flow_in)(a__test_conversion__coal__2005_01_01_00_00)
-1.0 variables(flow_cap)(a__test_conversion__coal)
<= 0.0

c_u_constraints(flow_in_max)(a__test_conversion__coal__2005_01_01_01_00)_:
+1 variables(flow_in)(a__test_conversion__coal__2005_01_01_01_00)
-1.0 variables(flow_cap)(a__test_conversion__coal)
<= 0.0

c_u_constraints(flow_in_max)(a__test_conversion__gas__2005_01_01_00_00)_:
+1 variables(flow_in)(a__test_conversion__gas__2005_01_01_00_00)
-1.0 variables(flow_cap)(a__test_conversion__gas)
<= 0.0

c_u_constraints(flow_in_max)(a__test_conversion__gas__2005_01_01_01_00)_:
+1 variables(flow_in)(a__test_conversion__gas__2005_01_01_01_00)
-1.0 variables(flow_cap)(a__test_conversion__gas)
<= 0.0

c_u_constraints(flow_in_max)(a__test_demand_coal__coal__2005_01_01_00_00)_:
+1 variables(flow_in)(a__test_demand_coal__coal__2005_01_01_00_00)
-1.0 variables(flow_cap)(a__test_demand_coal__coal)
<= 0.0

c_u_constraints(flow_in_max)(a__test_demand_coal__coal__2005_01_01_01_00)_:
-1.0 variables(flow_cap)(a__test_demand_coal__coal)
+1 variables(flow_in)(a__test_demand_coal__coal__2005_01_01_01_00)
<= 0.0

c_u_constraints(flow_in_max)(a__test_demand_elec__electricity__2005_01_01_00_00)_:
+1 variables(flow_in)(a__test_demand_elec__electricity__2005_01_01_00_00)
-1.0 variables(flow_cap)(a__test_demand_elec__electricity)
<= 0.0

c_u_constraints(flow_in_max)(a__test_demand_elec__electricity__2005_01_01_01_00)_:
-1.0 variables(flow_cap)(a__test_demand_elec__electricity)
+1 variables(flow_in)(a__test_demand_elec__electricity__2005_01_01_01_00)
<= 0.0

c_u_constraints(flow_in_max)(b__test_conversion__coal__2005_01_01_00_00)_:
+1 variables(flow_in)(b__test_conversion__coal__2005_01_01_00_00)
-1.0 variables(flow_cap)(b__test_conversion__coal)
<= 0.0

c_u_constraints(flow_in_max)(b__test_conversion__coal__2005_01_01_01_00)_:
+1 variables(flow_in)(b__test_conversion__coal__2005_01_01_01_00)
-1.0 variables(flow_cap)(b__test_conversion__coal)
<= 0.0

c_u_constraints(flow_in_max)(b__test_conversion__gas__2005_01_01_00_00)_:
+1 variables(flow_in)(b__test_conversion__gas__2005_01_01_00_00)
-1.0 variables(flow_cap)(b__test_conversion__gas)
<= 0.0

c_u_constraints(flow_in_max)(b__test_conversion__gas__2005_01_01_01_00)_:
+1 variables(flow_in)(b__test_conversion__gas__2005_01_01_01_00)
-1.0 variables(flow_cap)(b__test_conversion__gas)
<= 0.0

c_u_constraints(flow_in_max)(b__test_demand_coal__coal__2005_01_01_00_00)_:
+1 variables(flow_in)(b__test_demand_coal__coal__2005_01_01_00_00)
-1.0 variables(flow_cap)(b__test_demand_coal__coal)
<= 0.0

c_u_constraints(flow_in_max)(b__test_demand_coal__coal__2005_01_01_01_00)_:
-1.0 variables(flow_cap)(b__test_demand_coal__coal)
+1 variables(flow_in)(b__test_demand_coal__coal__2005_01_01_01_00)
<= 0.0

c_u_constraints(flow_in_max)(b__test_demand_elec__electricity__2005_01_01_00_00)_:
+1 variables(flow_in)(b__test_demand_elec__electricity__2005_01_01_00_00)
-1.0 variables(flow_cap)(b__test_demand_elec__electricity)
<= 0.0

c_u_constraints(flow_in_max)(b__test_demand_elec__electricity__2005_01_01_01_00)_:
-1.0 variables(flow_cap)(b__test_demand_elec__electricity)
+1 variables(flow_in)(b__test_demand_elec__electricity__2005_01_01_01_00)
<= 0.0

c_u_constraints(source_max)(a__test_supply_coal__2005_01_01_00_00)_:
+1 variables(source_use)(a__test_supply_coal__2005_01_01_00_00)
-1.0 variables(source_cap)(a__test_supply_coal)
<= 0.0

c_u_constraints(source_max)(a__test_supply_coal__2005_01_01_01_00)_:
-1.0 variables(source_cap)(a__test_supply_coal)
+1 variables(source_use)(a__test_supply_coal__2005_01_01_01_00)
<= 0.0

c_u_constraints(source_max)(a__test_supply_gas__2005_01_01_00_00)_:
+1 variables(source_use)(a__test_supply_gas__2005_01_01_00_00)
-1.0 variables(source_cap)(a__test_supply_gas)
<= 0.0

c_u_constraints(source_max)(a__test_supply_gas__2005_01_01_01_00)_:
-1.0 variables(source_cap)(a__test_supply_gas)
+1 variables(source_use)(a__test_supply_gas__2005_01_01_01_00)
<= 0.0

c_u_constraints(source_max)(b__test_supply_coal__2005_01_01_00_00)_:
+1 variables(source_use)(b__test_supply_coal__2005_01_01_00_00)
-1.0 variables(source_cap)(b__test_supply_coal)
<= 0.0

c_u_constraints(source_max)(b__test_supply_coal__2005_01_01_01_00)_:
-1.0 variables(source_cap)(b__test_supply_coal)
+1 variables(source_use)(b__test_supply_coal__2005_01_01_01_00)
<= 0.0

c_u_constraints(source_max)(b__test_supply_gas__2005_01_01_00_00)_:
+1 variables(source_use)(b__test_supply_gas__2005_01_01_00_00)
-1.0 variables(source_cap)(b__test_supply_gas)
<= 0.0

c_u_constraints(source_max)(b__test_supply_gas__2005_01_01_01_00)_:
-1.0 variables(source_cap)(b__test_supply_gas)
+1 variables(source_use)(b__test_supply_gas__2005_01_01_01_00)
<= 0.0

c_e_constraints(system_balance)(a__coal__2005_01_01_00_00)_:
-1 variables(flow_in)(a__test_conversion__coal__2005_01_01_00_00)
+1 variables(flow_out)(a__test_supply_coal__coal__2005_01_01_00_00)
-1 variables(flow_in)(a__test_demand_coal__coal__2005_01_01_00_00)
= 0.0

c_e_constraints(system_balance)(a__coal__2005_01_01_01_00)_:
-1 variables(flow_in)(a__test_conversion__coal__2005_01_01_01_00)
+1 variables(flow_out)(a__test_supply_coal__coal__2005_01_01_01_00)
-1 variables(flow_in)(a__test_demand_coal__coal__2005_01_01_01_00)
= 0.0

c_e_constraints(system_balance)(a__electricity__2005_01_01_00_00)_:
+1 variables(flow_out)(a__test_conversion__electricity__2005_01_01_00_00)
-1 variables(flow_in)(a__test_demand_elec__electricity__2005_01_01_00_00)
= 0.0

c_e_constraints(system_balance)(a__electricity__2005_01_01_01_00)_:
+1 variables(flow_out)(a__test_conversion__electricity__2005_01_01_01_00)
-1 variables(flow_in)(a__test_demand_elec__electricity__2005_01_01_01_00)
= 0.0

c_e_constraints(system_balance)(a__gas__2005_01_01_00_00)_:
-1 variables(flow_in)(a__test_conversion__gas__2005_01_01_00_00)
+1 variables(flow_out)(a__test_supply_gas__gas__2005_01_01_00_00)
= 0.0

c_e_constraints(system_balance)(a__gas__2005_01_01_01_00)_:
-1 variables(flow_in)(a__test_conversion__gas__2005_01_01_01_00)
+1 variables(flow_out)(a__test_supply_gas__gas__2005_01_01_01_00)
= 0.0

c_e_constraints(system_balance)(b__coal__2005_01_01_00_00)_:
-1 variables(flow_in)(b__test_conversion__coal__2005_01_01_00_00)
+1 variables(flow_out)(b__test_supply_coal__coal__2005_01_01_00_00)
-1 variables(flow_in)(b__test_demand_coal__coal__2005_01_01_00_00)
= 0.0

c_e_constraints(system_balance)(b__coal__2005_01_01_01_00)_:
-1 variables(flow_in)(b__test_conversion__coal__2005_01_01_01_00)
+1 variables(flow_out)(b__test_supply_coal__coal__2005_01_01_01_00)
-1 variables(flow_in)(b__test_demand_coal__coal__2005_01_01_01_00)
= 0.0

c_e_constraints(system_balance)(b__electricity__2005_01_01_00_00)_:
+1 variables(flow_out)(b__test_conversion__electricity__2005_01_01_00_00)
-1 variables(flow_in)(b__test_demand_elec__electricity__2005_01_01_00_00)
= 0.0

c_e_constraints(system_balance)(b__electricity__2005_01_01_01_00)_:
+1 variables(flow_out)(b__test_conversion__electricity__2005_01_01_01_00)
-1 variables(flow_in)(b__test_demand_elec__electricity__2005_01_01_01_00)
= 0.0

c_e_constraints(system_balance)(b__gas__2005_01_01_00_00)_:
-1 variables(flow_in)(b__test_conversion__gas__2005_01_01_00_00)
+1 variables(flow_out)(b__test_supply_gas__gas__2005_01_01_00_00)
= 0.0

c_e_constraints(system_balance)(b__gas__2005_01_01_01_00)_:
-1 variables(flow_in)(b__test_conversion__gas__2005_01_01_01_00)
+1 variables(flow_out)(b__test_supply_gas__gas__2005_01_01_01_00)
= 0.0

c_e_constraints(balance_demand)(a__test_demand_elec__electricity__2005_01_01_00_00)_:
+1 variables(flow_in)(a__test_demand_elec__electricity__2005_01_01_00_00)
= 5.0

c_e_constraints(balance_demand)(a__test_demand_elec__electricity__2005_01_01_01_00)_:
+1 variables(flow_in)(a__test_demand_elec__electricity__2005_01_01_01_00)
= 5.0

c_e_constraints(balance_demand)(b__test_demand_elec__electricity__2005_01_01_00_00)_:
+1 variables(flow_in)(b__test_demand_elec__electricity__2005_01_01_00_00)
= 5.0

c_e_constraints(balance_demand)(b__test_demand_elec__electricity__2005_01_01_01_00)_:
+1 variables(flow_in)(b__test_demand_elec__electricity__2005_01_01_01_00)
= 5.0

c_u_constraints(balance_demand)(a__test_demand_coal__coal__2005_01_01_00_00)_:
+1 variables(flow_in)(a__test_demand_coal__coal__2005_01_01_00_00)
<= 10.0

c_u_constraints(balance_demand)(a__test_demand_coal__coal__2005_01_01_01_00)_:
+1 variables(flow_in)(a__test_demand_coal__coal__2005_01_01_01_00)
<= 10.0

c_u_constraints(balance_demand)(b__test_demand_coal__coal__2005_01_01_00_00)_:
+1 variables(flow_in)(b__test_demand_coal__coal__2005_01_01_00_00)
<= 10.0

c_u_constraints(balance_demand)(b__test_demand_coal__coal__2005_01_01_01_00)_:
+1 variables(flow_in)(b__test_demand_coal__coal__2005_01_01_01_00)
<= 10.0

c_e_constraints(balance_supply_no_storage)(a__test_supply_coal__coal__2005_01_01_00_00)_:
+1.1111111111111112 variables(flow_out)(a__test_supply_coal__coal__2005_01_01_00_00)
-1 variables(source_use)(a__test_supply_coal__2005_01_01_00_00)
= 0.0

c_e_constraints(balance_supply_no_storage)(a__test_supply_coal__coal__2005_01_01_01_00)_:
+1.1111111111111112 variables(flow_out)(a__test_supply_coal__coal__2005_01_01_01_00)
-1 variables(source_use)(a__test_supply_coal__2005_01_01_01_00)
= 0.0

c_e_constraints(balance_supply_no_storage)(a__test_supply_gas__gas__2005_01_01_00_00)_:
+1.1111111111111112 variables(flow_out)(a__test_supply_gas__gas__2005_01_01_00_00)
-1 variables(source_use)(a__test_supply_gas__2005_01_01_00_00)
= 0.0

c_e_constraints(balance_supply_no_storage)(a__test_supply_gas__gas__2005_01_01_01_00)_:
+1.1111111111111112 variables(flow_out)(a__test_supply_gas__gas__2005_01_01_01_00)
-1 variables(source_use)(a__test_supply_gas__2005_01_01_01_00)
= 0.0

c_e_constraints(balance_supply_no_storage)(b__test_supply_coal__coal__2005_01_01_00_00)_:
+1.1111111111111112 variables(flow_out)(b__test_supply_coal__coal__2005_01_01_00_00)
-1 variables(source_use)(b__test_supply_coal__2005_01_01_00_00)
= 0.0

c_e_constraints(balance_supply_no_storage)(b__test_supply_coal__coal__2005_01_01_01_00)_:
+1.1111111111111112 variables(flow_out)(b__test_supply_coal__coal__2005_01_01_01_00)
-1 variables(source_use)(b__test_supply_coal__2005_01_01_01_00)
= 0.0

c_e_constraints(balance_supply_no_storage)(b__test_supply_gas__gas__2005_01_01_00_00)_:
+1.1111111111111112 variables(flow_out)(b__test_supply_gas__gas__2005_01_01_00_00)
-1 variables(source_use)(b__test_supply_gas__2005_01_01_00_00)
= 0.0

c_e_constraints(balance_supply_no_storage)(b__test_supply_gas__gas__2005_01_01_01_00)_:
+1.1111111111111112 variables(flow_out)(b__test_supply_gas__gas__2005_01_01_01_00)
-1 variables(source_use)(b__test_supply_gas__2005_01_01_01_00)
= 0.0

bounds
   0.0 <= variables(flow_in)(a__test_conversion__coal__2005_01_01_00_00) <= +inf
   0.0 <= variables(flow_in)(a__test_conversion__gas__2005_01_01_00_00) <= +inf
   0.0 <= variables(flow_out)(a__test_conversion__electricity__2005_01_01_00_00) <= +inf
   0.0 <= variables(flow_in)(a__test_conversion__coal__2005_01_01_01_00) <= +inf
   0.0 <= variables(flow_in)(a__test_conversion__gas__2005_01_01_01_00) <= +inf
   0.0 <= variables(flow_out)(a__test_conversion__electricity__2005_01_01_01_00) <= +inf
   0.0 <= variables(flow_out)(b__test_conversion__electricity__2005_01_01_00_00) <= +inf
   0.0 <= variables(flow_in)(b__test_conversion__coal__2005_01_01_00_00) <= +inf
   0.0 <= variables(flow_in)(b__test_conversion__gas__2005_01_01_00_00) <= +inf
   0.0 <= variables(flow_out)(b__test_conversion__electricity__2005_01_01_01_00) <= +inf
   0.0 <= variables(flow_in)(b__test_conversion__coal__2005_01_01_01_00) <= +inf
   0.0 <= variables(flow_in)(b__test_conversion__gas__2005_01_01_01_00) <= +inf
   0.0 <= variables(flow_cap)(a__test_conversion__electricity) <= +inf
   0.0 <= variables(flow_out)(a__test_supply_coal__coal__2005_01_01_00_00) <= +inf
   0.0 <= variables(flow_cap)(a__test_supply_coal__coal) <= 10.0
   0.0 <= variables(flow_out)(a__test_supply_coal__coal__2005_01_01_01_00) <= +inf
   0.0 <= variables(flow_out)(a__test_supply_gas__gas__2005_01_01_00_00) <= +inf
   0.0 <= variables(flow_cap)(a__test_supply_gas__gas) <= 15.0
   0.0 <= variables(flow_out)(a__test_supply_gas__gas__2005_01_01_01_00) <= +inf
   0.0 <= variables(flow_cap)(b__test_conversion__electricity) <= +inf
   0.0 <= variables(flow_out)(b__test_supply_coal__coal__2005_01_01_00_00) <= +inf
   0.0 <= variables(flow_cap)(b__test_supply_coal__coal) <= 10.0
   0.0 <= variables(flow_out)(b__test_supply_coal__coal__2005_01_01_01_00) <= +inf
   0.0 <= variables(flow_out)(b__test_supply_gas__gas__2005_01_01_00_00) <= +inf
   0.0 <= variables(flow_cap)(b__test_supply_gas__gas) <= 15.0
   0.0 <= variables(flow_out)(b__test_supply_gas__gas__2005_01_01_01_00) <= +inf
   0.0 <= variables(flow_cap)(a__test_conversion__coal) <= +inf
   0.0 <= variables(flow_cap)(a__test_conversion__gas) <= +inf
   0.0 <= variables(flow_in)(a__test_demand_coal__coal__2005_01_01_00_00) <= +inf
   0.0 <= variables(flow_cap)(a__test_demand_coal__coal) <= +inf
   0.0 <= variables(flow_in)(a__test_demand_coal__coal__2005_01_01_01_00) <= +inf
   0.0 <= variables(flow_in)(a__test_demand_elec__electricity__2005_01_01_00_00) <= +inf
   0.0 <= variables(flow_cap)(a__test_demand_elec__electricity) <= +inf
   0.0 <= variables(flow_in)(a__test_demand_elec__electricity__2005_01_01_01_00) <= +inf
   0.0 <= variables(flow_cap)(b__test_conversion__coal) <= +inf
   0.0 <= variables(flow_cap)(b__test_conversion__gas) <= +inf
   0.0 <= variables(flow_in)(b__test_demand_coal__coal__2005_01_01_00_00) <= +inf
   0.0 <= variables(flow_cap)(b__test_demand_coal__coal) <= +inf
   0.0 <= variables(flow_in)(b__test_demand_coal__coal__2005_01_01_01_00) <= +inf
   0.0 <= variables(flow_in)(b__test_demand_elec__electricity__2005_01_01_00_00) <= +inf
   0.0 <= variables(flow_cap)(b__test_demand_elec__electricity) <= +inf
   0.0 <= variables(flow_in)(b__test_demand_elec__electricity__2005_01_01_01_00) <= +inf
   0.0 <= variables(source_use)(a__test_supply_coal__2005_01_01_00_00) <= +inf
   0.0 <= variables(source_cap)(a__test_supply_coal) <= +inf
   0.0 <= variables(source_use)(a__test_supply_coal__2005_01_01_01_00) <= +inf
   0.0 <= variables(source_use)(a__test_supply_gas__2005_01_01_00_00) <= +inf
   0.0 <= variables(source_cap)(a__test_supply_gas) <= +inf
   0.0 <= variables(source_use)(a__test_supply_gas__2005_01_01_01_00) <= +inf
   0.0 <= variables(source_use)(b__test_supply_coal__2005_01_01_00_00) <= +inf
   0.0 <= variables(source_cap)(b__test_supply_coal) <= +inf
   0.0 <= variables(source_use)(b__test_supply_coal__2005_01_01_01_00) <= +inf
   0.0 <= variables(source_use)(b__test_supply_gas__2005_01_01_00_00) <= +inf
   0.0 <= variables(source_cap)(b__test_supply_gas) <= +inf
   0.0 <= variables(source_use)(b__test_supply_gas__2005_01_01_01_00) <= +inf
end
